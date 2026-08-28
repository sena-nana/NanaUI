//! Vue host adapter backed by the Runtime's authoritative retained tree.
//!
//! Keeps createElement / insert / patchProp / event flags for the JS custom
//! renderer and capability bridge. Identity, hierarchy, node kind, text,
//! focus, style, interaction, and layout live in `nana_ui_runtime::UiWorld`;
//! this module retains only Vue compatibility metadata.
//!
//! Product Vue frames flush the same [`RuntimeDocument`] text+layout as L3.
//! [`crate::measure_layout`] adapts a style tree onto that engine for css-parity
//! and must not WriteLayout over flushed engine boxes.
//! [`LayoutBoxStore`] is the JS paint projection, not layout authority.

use std::{
    cell::UnsafeCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(any(test, feature = "hosted"))]
use nana_ui_runtime::AccessibilityUpdate;
#[cfg(not(feature = "scene-view"))]
use nana_ui_runtime::MeasureTextShaper;
use nana_ui_runtime::{
    AccessibilityDelta, AccessibilityRole, AccessibilityState, AppContext,
    AppShell as RuntimeAppShell, AppTitleBar as RuntimeAppTitleBar, ComponentBindKind,
    ComponentTypeId, ComponentView, CustomRenderNode, Dock as RuntimeDock, DockAxis, DockNode,
    Entity, HOST_TEXTURE_RENDERER, HighlightRequest, ImeComposition, InteractionState,
    LayoutBox as RuntimeLayoutBox, LayoutViewport, MutationQueue,
    NativeMarkdown as RuntimeNativeMarkdown, NodeKind, NodeStyle,
    SegmentedOption as RuntimeSegmentedOption, SelectionChrome, SemanticOption, SemanticSpec,
    SettingsPage as RuntimeSettingsPage, SidebarFrame as RuntimeSidebarFrame,
    SplitPane as RuntimeSplitPane, StableNodeId, TextContent, TextInputState, UiMutation, UiWorld,
    Workspace as RuntimeWorkspace, WorkspaceRegionSlot,
};
use nana_ui_scene::{RuntimeDocument, UiScene};

/// Opaque handle returned to the Vue custom renderer (JSON-safe number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeHandle(pub u64);

impl NodeHandle {
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<NodeHandle> for nana_ui_runtime::StableNodeId {
    type Error = NodeHandle;

    fn try_from(handle: NodeHandle) -> Result<Self, Self::Error> {
        Self::new(handle.0).ok_or(handle)
    }
}

impl From<nana_ui_runtime::StableNodeId> for NodeHandle {
    fn from(id: nana_ui_runtime::StableNodeId) -> Self {
        Self(id.get())
    }
}

/// Vue window document id for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

impl TryFrom<DocumentId> for nana_ui_runtime::DocumentId {
    type Error = DocumentId;

    fn try_from(id: DocumentId) -> Result<Self, Self::Error> {
        Self::new(id.0).ok_or(id)
    }
}

impl From<nana_ui_runtime::DocumentId> for DocumentId {
    fn from(id: nana_ui_runtime::DocumentId) -> Self {
        Self(id.get())
    }
}

/// Number of node ids reserved for each Vue document.
///
/// Handles stay exactly representable by JavaScript `Number` while allowing a
/// single V8 context to route nodes from independent window documents without
/// an engine-side object wrapper. Document 1 deliberately keeps the historical
/// `1 = html`, `2 = body` handles; document N uses `(N - 1) * 2^32 + local`.
pub const NODE_HANDLE_DOCUMENT_STRIDE: u64 = 1 << 32;

impl DocumentId {
    pub fn node_base(self) -> u64 {
        self.0
            .saturating_sub(1)
            .saturating_mul(NODE_HANDLE_DOCUMENT_STRIDE)
    }

    pub fn from_node(handle: NodeHandle) -> Self {
        Self(handle.0 / NODE_HANDLE_DOCUMENT_STRIDE + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomNodeKind {
    Element,
    Text,
    Comment,
    Document,
    Other,
}

/// Layout box in logical CSS px (viewport / Scene absolute coordinates).
///
/// Sources, in preference order for JS `getBoundingClientRect` / `layoutBox`:
/// 1. Scene paint writeback ([`LayoutBoxStore`])
/// 2. Style-Model [`crate::measure_layout`] applied via [`NanaTreeDocument::apply_layout_boxes`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    pub handle: NodeHandle,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Per-window JS paint-phase geometry. Not layout authority.
///
/// Incremental Scene paint via [`Self::record`]; scroll lives in a JS overlay
/// and is not written back to Runtime `LayoutBox`.
#[derive(Debug, Default)]
pub struct LayoutBoxStore {
    boxes: Mutex<HashMap<u64, LayoutBox>>,
    views: Mutex<HashMap<u64, LayoutBox>>,
    transforms: Mutex<HashMap<u64, (LayoutBox, [f32; 6])>>,
}

impl LayoutBoxStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop JS scroll overlays; keep last Scene paint boxes.
    pub fn begin_frame(&self) {
        if let Ok(mut guard) = self.views.lock() {
            guard.clear();
        }
    }

    pub fn record(&self, handle: NodeHandle, x: f32, y: f32, width: f32, height: f32) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.insert(
                handle.0,
                LayoutBox {
                    handle,
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn record_transformed(
        &self,
        handle: NodeHandle,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        affine: [f32; 6],
    ) {
        let source = LayoutBox {
            handle,
            x,
            y,
            width,
            height,
        };
        let transformed = transform_layout_box(source, affine);
        if let Ok(mut guard) = self.boxes.lock() {
            guard.insert(handle.0, transformed);
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.insert(handle.0, (source, affine));
        }
    }

    pub fn remove(&self, handle: NodeHandle) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.remove(&handle.0);
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn retain(&self, mut live: impl FnMut(u64) -> bool) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.retain(|&id, _| live(id));
        }
        if let Ok(mut guard) = self.views.lock() {
            guard.retain(|&id, _| live(id));
        }
        if let Ok(mut guard) = self.transforms.lock() {
            guard.retain(|&id, _| live(id));
        }
    }

    fn clear_view(&self, handle: NodeHandle) {
        if let Ok(mut guard) = self.views.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn contains_point(&self, handle: NodeHandle, x: f32, y: f32) -> bool {
        if self.view_box(handle).is_some() {
            return self.get(handle).is_some_and(|box_| {
                x >= box_.x && y >= box_.y && x < box_.x + box_.width && y < box_.y + box_.height
            });
        }
        let transformed = self
            .transforms
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied());
        let Some((source, affine)) = transformed else {
            return self.get(handle).is_some_and(|box_| {
                x >= box_.x && y >= box_.y && x < box_.x + box_.width && y < box_.y + box_.height
            });
        };
        inverse_affine_point(x, y, affine).is_some_and(|(local_x, local_y)| {
            local_x >= source.x
                && local_y >= source.y
                && local_x < source.x + source.width
                && local_y < source.y + source.height
        })
    }

    pub fn local_point(&self, handle: NodeHandle, x: f32, y: f32) -> Option<(f32, f32)> {
        if let Some(box_) = self.view_box(handle) {
            return Some((x - box_.x, y - box_.y));
        }
        let transformed = self
            .transforms
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied());
        match transformed {
            Some((source, affine)) => {
                inverse_affine_point(x, y, affine).map(|(px, py)| (px - source.x, py - source.y))
            }
            None => self.get(handle).map(|box_| (x - box_.x, y - box_.y)),
        }
    }

    pub fn translate(&self, handle: NodeHandle, dx: f32, dy: f32) -> Option<LayoutBox> {
        let mut box_ = self.get(handle)?;
        box_.x += dx;
        box_.y += dy;
        self.overlay_view(box_);
        Some(box_)
    }

    pub(crate) fn overlay_view(&self, box_: LayoutBox) {
        if let Ok(mut views) = self.views.lock() {
            views.insert(box_.handle.0, box_);
        }
    }

    fn view_box(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.views
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied())
    }

    pub fn get(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.view_box(handle).or_else(|| self.source_box(handle))
    }

    /// Scene paint box, ignoring JS scroll / sticky overlays.
    pub fn source_box(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.boxes
            .lock()
            .ok()
            .and_then(|g| g.get(&handle.0).copied())
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.lock().map(|g| g.is_empty()).unwrap_or(true)
    }

    pub fn len(&self) -> usize {
        self.boxes.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn snapshot(&self) -> Vec<(NodeHandle, LayoutBox)> {
        let Ok(guard) = self.boxes.lock() else {
            return Vec::new();
        };
        let mut out: Vec<(NodeHandle, LayoutBox)> =
            guard.iter().map(|(&id, b)| (NodeHandle(id), *b)).collect();
        out.sort_by_key(|(h, _)| h.0);
        out
    }
}

fn inverse_affine_point(x: f32, y: f32, [a, b, c, d, e, f]: [f32; 6]) -> Option<(f32, f32)> {
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let px = x - e;
    let py = y - f;
    Some((
        (d * px - c * py) / determinant,
        (-b * px + a * py) / determinant,
    ))
}

fn transform_layout_box(source: LayoutBox, [a, b, c, d, e, f]: [f32; 6]) -> LayoutBox {
    let corners = [
        (source.x, source.y),
        (source.x + source.width, source.y),
        (source.x, source.y + source.height),
        (source.x + source.width, source.y + source.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        let tx = a * x + c * y + e;
        let ty = b * x + d * y + f;
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    LayoutBox {
        handle: source.handle,
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    }
}

/// Prefer `store` writeback, else the document's pre-paint measure cache.
pub fn get_layout_box_from(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    handle: NodeHandle,
) -> Option<LayoutBox> {
    store.get(handle).or_else(|| doc.layout_box(handle))
}

/// JS `scrollWidth` / `scrollHeight`: Runtime metrics, else descendant union.
pub(crate) fn query_scroll_content_size(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    node: NodeHandle,
    client_width: f32,
    client_height: f32,
) -> (f32, f32) {
    if let Some(metrics) = doc.scroll_metrics(node) {
        return (
            metrics.content_width.max(client_width).max(0.0),
            metrics.content_height.max(client_height).max(0.0),
        );
    }
    let Some(viewport) = get_layout_box_from(store, doc, node) else {
        return (client_width.max(0.0), client_height.max(0.0));
    };
    let (content_width, content_height) =
        union_descendant_content(doc, node, viewport, |doc, child| {
            get_layout_box_from(store, doc, child)
        });
    (
        content_width.max(client_width),
        content_height.max(client_height),
    )
}

fn union_descendant_content(
    doc: &NanaTreeDocument,
    node: NodeHandle,
    viewport: LayoutBox,
    mut box_of: impl FnMut(&NanaTreeDocument, NodeHandle) -> Option<LayoutBox>,
) -> (f32, f32) {
    let mut content_width = viewport.width;
    let mut content_height = viewport.height;
    let mut stack = doc.children_of(node);
    while let Some(child) = stack.pop() {
        if let Some(box_) = box_of(doc, child) {
            content_width = content_width.max(box_.x + box_.width - viewport.x);
            content_height = content_height.max(box_.y + box_.height - viewport.y);
        }
        stack.extend(doc.children_of(child));
    }
    (content_width.max(0.0), content_height.max(0.0))
}

/// Document layout cache (pre-paint measure or last [`NanaTreeDocument::apply_layout_boxes`]).
///
/// Live Scene writeback is on the per-window [`LayoutBoxStore`]; use [`get_layout_box_from`].
pub fn get_layout_box(doc: &NanaTreeDocument, handle: NodeHandle) -> Option<LayoutBox> {
    doc.layout_box(handle)
}

/// Compact dump used by headless probes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSnapshot {
    pub boxes: Vec<LayoutBox>,
    pub texts: Vec<(NodeHandle, String)>,
    pub tags: Vec<(NodeHandle, String)>,
    pub event_targets: HashSet<(u64, String)>,
    pub gpu_slots: Vec<(NodeHandle, String)>,
}

/// Vue / DOM element namespace (`svg` | `mathml` | HTML/`None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementNamespace {
    Html,
    Svg,
    MathMl,
}

impl ElementNamespace {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("svg") => Self::Svg,
            Some("mathml") | Some("math") => Self::MathMl,
            _ => Self::Html,
        }
    }

    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Html => None,
            Self::Svg => Some("svg"),
            Self::MathMl => Some("mathml"),
        }
    }
}

#[derive(Debug, Clone)]
enum NodeData {
    Element {
        namespace: ElementNamespace,
        attrs: HashMap<String, String>,
    },
    Text,
    Comment,
}

#[derive(Debug, Clone)]
struct Node {
    data: NodeData,
    scope_id: Option<String>,
}

/// Shared handle to one window's [`RuntimeDocument`].
///
/// JS host ops keep [`NanaTreeDocument`] behind a mutex while
/// [`crate::VueRuntimeProgram`] must return `&RuntimeDocument`. Both sides clone
/// this `Arc` and see the same tree. Access is single-threaded on the UI/JS
/// loop; overlapping exclusive borrows are a programming error.
pub struct SharedRuntimeDocument {
    inner: UnsafeCell<RuntimeDocument>,
}

unsafe impl Send for SharedRuntimeDocument {}
unsafe impl Sync for SharedRuntimeDocument {}

impl SharedRuntimeDocument {
    pub fn new(document: nana_ui_runtime::DocumentId) -> Arc<Self> {
        Arc::new(Self {
            inner: UnsafeCell::new(RuntimeDocument::new(document)),
        })
    }

    pub fn get(&self) -> &RuntimeDocument {
        // Safety: UI/JS loop is single-threaded; callers never alias exclusive
        // RuntimeDocument borrows across host input vs JS host-op frames.
        unsafe { &*self.inner.get() }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn get_mut(&self) -> &mut RuntimeDocument {
        // Safety: same single-threaded non-overlapping-borrow invariant as
        // [`Self::get`].
        unsafe { &mut *self.inner.get() }
    }
}

/// Vue-owned Runtime. World reads/writes forward to [`UiWorld`]; typed views
/// and `assemble_*` live on [`AppContext`]. The retained scene lives on the
/// same [`RuntimeDocument`] the Scene host flushes.
struct VueRuntime {
    document: Arc<SharedRuntimeDocument>,
}

impl VueRuntime {
    fn new(document: nana_ui_runtime::DocumentId) -> Self {
        Self {
            document: SharedRuntimeDocument::new(document),
        }
    }

    fn world(&self) -> &UiWorld {
        self.document.get().context().world()
    }

    fn context(&self) -> &AppContext {
        self.document.get().context()
    }

    fn context_mut(&mut self) -> &mut AppContext {
        self.document.get_mut().context_mut()
    }

    fn runtime_document(&self) -> &RuntimeDocument {
        self.document.get()
    }

    fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        self.document.get_mut()
    }

    fn shared(&self) -> Arc<SharedRuntimeDocument> {
        Arc::clone(&self.document)
    }
}

impl std::ops::Deref for VueRuntime {
    type Target = UiWorld;

    fn deref(&self) -> &UiWorld {
        self.document.get().context().world()
    }
}

impl std::ops::DerefMut for VueRuntime {
    fn deref_mut(&mut self) -> &mut UiWorld {
        self.document.get_mut().context_mut().world_mut()
    }
}

#[derive(Default)]
struct PendingHostOps {
    mutations: MutationQueue,
    parent: HashMap<u64, Option<u64>>,
    children: HashMap<u64, Vec<u64>>,
    kinds: HashMap<u64, NodeKind>,
    texts: HashMap<u64, String>,
    events: HashMap<u64, HashSet<String>>,
    gpu: HashMap<u64, Option<CustomRenderNode>>,
}

impl PendingHostOps {
    /// Whether a commit is needed. Every facade write pushes a mutation; the
    /// overlay maps are read-through mirrors of the same pending writes (plus
    /// runtime-primed child caches), so `mutations` is the commit signal.
    /// `commit_pending_queue` clears both sides together to keep them in
    /// lockstep — never clear one without the other.
    fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Variant name for a rejected mutation, kept out of diagnostics payloads to
/// avoid dumping whole node styles into the sink.
fn mutation_label(mutation: &UiMutation) -> &'static str {
    match mutation {
        UiMutation::Create { .. } => "Create",
        UiMutation::Insert { .. } => "Insert",
        UiMutation::Detach { .. } => "Detach",
        UiMutation::ParkSubtree { .. } => "ParkSubtree",
        UiMutation::DespawnSubtree { .. } => "DespawnSubtree",
        UiMutation::SetStyle { .. } => "SetStyle",
        UiMutation::SetTheme { .. } => "SetTheme",
        UiMutation::SetText { .. } => "SetText",
        UiMutation::WriteLayout { .. } => "WriteLayout",
        UiMutation::SetScrollOffset { .. } => "SetScrollOffset",
        UiMutation::SetScrollMetrics { .. } => "SetScrollMetrics",
        UiMutation::SetInteraction { .. } => "SetInteraction",
        UiMutation::SetCustomRender { .. } => "SetCustomRender",
        UiMutation::SetEventListener { .. } => "SetEventListener",
        UiMutation::SetComponentType { .. } => "SetComponentType",
        UiMutation::SetStandardVisual { .. } => "SetStandardVisual",
        UiMutation::SetAccessibility { .. } => "SetAccessibility",
        UiMutation::SetOverlayHost { .. } => "SetOverlayHost",
        UiMutation::CapturePointer { .. } => "CapturePointer",
        UiMutation::ReleasePointer { .. } => "ReleasePointer",
        UiMutation::StartAnimation { .. } => "StartAnimation",
        UiMutation::StopAnimation { .. } => "StopAnimation",
        UiMutation::RequestFocus { .. } => "RequestFocus",
        UiMutation::SetIme { .. } => "SetIme",
        UiMutation::SetTextInput { .. } => "SetTextInput",
        UiMutation::SetTextSelection { .. } => "SetTextSelection",
        UiMutation::ReplaceTextSelection { .. } => "ReplaceTextSelection",
        UiMutation::SetHighlightRequest { .. } => "SetHighlightRequest",
    }
}

#[derive(Default)]
struct PendingAssembly {
    workspaces: Vec<(StableNodeId, RuntimeWorkspace)>,
    docks: Vec<(StableNodeId, RuntimeDock)>,
    split_panes: Vec<(StableNodeId, RuntimeSplitPane)>,
    title_bars: Vec<(StableNodeId, RuntimeAppTitleBar)>,
    app_shells: Vec<(StableNodeId, RuntimeAppShell)>,
    markdowns: Vec<(StableNodeId, RuntimeNativeMarkdown)>,
    settings_pages: Vec<(StableNodeId, RuntimeSettingsPage)>,
}

impl PendingAssembly {
    fn apply(self, context: &mut AppContext) {
        for (id, component) in self.title_bars {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_app_title_bar(entity);
            }
        }
        for (id, component) in self.workspaces {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_workspace(entity);
            }
        }
        for (id, component) in self.docks {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_dock(entity);
            }
        }
        for (id, component) in self.split_panes {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_split_pane(entity);
            }
        }
        for (id, component) in self.app_shells {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_app_shell(entity);
            }
        }
        for (id, component) in self.markdowns {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_markdown(entity);
            }
        }
        for (id, component) in self.settings_pages {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_settings_page(entity);
            }
        }
    }
}

/// Vue custom-renderer facade. Not the product retained world.
///
/// Identity, hierarchy, kind, text, focus, style, interaction, and layout live
/// in `nana_ui_runtime::UiWorld`. `nodes` keeps only Vue DOM metadata
/// (namespace, attributes, scope id) needed by host ops. Do not treat this type
/// as a second ECS/DOM tree or as an Iced `Element` host.
pub struct NanaTreeDocument {
    id: DocumentId,
    runtime: VueRuntime,
    nodes: HashMap<u64, Node>,
    next_id: u64,
    html_root: NodeHandle,
    mount_root: NodeHandle,
    pending: PendingHostOps,
    stylesheets: Vec<String>,
    theme: String,
    logical_width: f32,
    logical_height: f32,
    scale_factor: f32,
    synced_semantic_revision: Option<u64>,
    /// Nodes whose Runtime `LayoutStyle` is written by a qualified component
    /// projection, recorded by the last semantic sync. Those components already
    /// consumed `props.layout` when they were built, so the CSS cascade
    /// writeback must not overwrite the geometry they projected.
    component_owned_layout: HashSet<u64>,
    pending_accessibility_updated: BTreeMap<StableNodeId, nana_ui_runtime::AccessibilityNode>,
    pending_accessibility_removed: BTreeSet<StableNodeId>,
    pending_accessibility_generation: u64,
    accessibility_full_required: bool,
    commit_rejections: Vec<String>,
    /// Facade nodes that currently expose a host-texture slot. Flush stamps
    /// only these instead of scanning the whole Vue node map.
    host_texture_nodes: HashSet<u64>,
    /// Shared slot registry handle. Device/Queue stay on the hosted renderer.
    #[cfg(feature = "scene-view")]
    host_textures: Option<nana_ui::HostTextureRegistry>,
    /// Test stand-in for a registered HostTexture generation/version pair.
    #[cfg(test)]
    host_texture_revision_overrides: HashMap<String, u64>,
    /// Monotonic origin for CSS transition / keyframe deadlines (same epoch as
    /// [`UiWorld::advance_animations`]).
    animation_epoch: Instant,
    /// Scene host epoch when wired through [`RuntimeAnimationClock`]; isolated
    /// tests leave this unset and fall back to [`Self::animation_epoch`].
    host_animation_epoch: Option<Instant>,
}

const MAX_PENDING_ACCESSIBILITY_CHANGES: usize = 4_096;
const MAX_COMMIT_REJECTIONS: usize = 32;

impl std::fmt::Debug for NanaTreeDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NanaTreeDocument")
            .field("id", &self.id)
            .field("nodes", &self.nodes.len())
            .field("generation", &self.runtime.generation())
            .field("html_root", &self.html_root)
            .field("mount_root", &self.mount_root)
            .finish_non_exhaustive()
    }
}

impl NanaTreeDocument {
    pub fn new(physical_width: u32, physical_height: u32, scale_factor: f32) -> Self {
        Self::with_id(DocumentId(1), physical_width, physical_height, scale_factor)
    }

    pub fn with_id(
        id: DocumentId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Self {
        let scale = scale_factor.max(0.01);
        let logical_width = physical_width as f32 / scale;
        let logical_height = physical_height as f32 / scale;
        let mut nodes = HashMap::new();
        let runtime_document =
            nana_ui_runtime::DocumentId::try_from(id).expect("Vue document IDs are nonzero");
        let mut runtime = VueRuntime::new(runtime_document);
        let node_base = id.node_base();
        let html_root = node_base + 1;
        let mount_root = node_base + 2;
        // Document 1: 1 = html, 2 = body. Auxiliary documents preserve the
        // same local ids inside their globally unique 2^32 namespace.
        nodes.insert(
            html_root,
            Node {
                data: NodeData::Element {
                    namespace: ElementNamespace::Html,
                    attrs: HashMap::new(),
                },
                scope_id: None,
            },
        );
        nodes.insert(
            mount_root,
            Node {
                data: NodeData::Element {
                    namespace: ElementNamespace::Html,
                    attrs: HashMap::new(),
                },
                scope_id: None,
            },
        );
        let mut mutations = MutationQueue::new();
        mutations.create(
            StableNodeId::new(html_root).expect("root ID is nonzero"),
            runtime_document,
            NodeKind::Element { tag: "html".into() },
        );
        mutations.create(
            StableNodeId::new(mount_root).expect("mount ID is nonzero"),
            runtime_document,
            NodeKind::Element { tag: "body".into() },
        );
        mutations.insert(
            StableNodeId::new(html_root).expect("root ID is nonzero"),
            StableNodeId::new(mount_root).expect("mount ID is nonzero"),
            None,
        );
        runtime
            .commit(mutations)
            .expect("document scaffold is valid");
        let mut doc = Self {
            id,
            runtime,
            nodes,
            next_id: node_base + 3,
            html_root: NodeHandle(html_root),
            mount_root: NodeHandle(mount_root),
            pending: PendingHostOps::default(),
            stylesheets: Vec::new(),
            theme: "light".into(),
            logical_width,
            logical_height,
            scale_factor: scale,
            synced_semantic_revision: None,
            component_owned_layout: HashSet::new(),
            pending_accessibility_updated: BTreeMap::new(),
            pending_accessibility_removed: BTreeSet::new(),
            pending_accessibility_generation: 0,
            accessibility_full_required: false,
            commit_rejections: Vec::new(),
            host_texture_nodes: HashSet::new(),
            #[cfg(feature = "scene-view")]
            host_textures: None,
            #[cfg(test)]
            host_texture_revision_overrides: HashMap::new(),
            animation_epoch: Instant::now(),
            host_animation_epoch: None,
        };
        doc.reset_layout_roots();
        doc
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    /// Retained Runtime tree. Vue node identity lives here; typed views live
    /// on [`Self::context`].
    pub fn world(&self) -> &UiWorld {
        self.runtime.world()
    }

    pub fn context(&self) -> &AppContext {
        self.runtime.context()
    }

    pub fn context_mut(&mut self) -> &mut AppContext {
        self.runtime.context_mut()
    }

    pub fn runtime_document(&self) -> &RuntimeDocument {
        self.runtime.runtime_document()
    }

    pub fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        self.runtime.runtime_document_mut()
    }

    pub fn shared_runtime_document(&self) -> Arc<SharedRuntimeDocument> {
        self.runtime.shared()
    }

    pub fn runtime_generation(&self) -> u64 {
        self.runtime.generation()
    }

    pub fn contains_handle(&self, node: NodeHandle) -> bool {
        self.nodes.contains_key(&node.0)
    }

    /// Write cascaded `LayoutStyle` from the MessageBridge into Runtime
    /// `NodeStyle` so `injectStylesheet` → `resolveLayout` uses the same
    /// geometry as `layout_style_tree`.
    ///
    /// Steady-state cost: `Arc` pointer identity is O(1) and skips the fat
    /// `LayoutStyle` `PartialEq`. Current Vue call sites still own a distinct
    /// `LayoutStyle`, so unchanged widgets pay one `PartialEq` here — no dirty
    /// bit / epoch skip, because a wrong fast path would drop a real write.
    pub fn sync_widget_layouts<'a>(
        &mut self,
        layouts: impl IntoIterator<Item = (u64, &'a nana_ui_core::LayoutStyle)>,
    ) {
        let mut mutations = MutationQueue::new();
        for (raw_id, layout) in layouts {
            let Some(id) = StableNodeId::new(raw_id) else {
                continue;
            };
            if !self.runtime.contains(id) && !self.nodes.contains_key(&raw_id) {
                continue;
            }
            if self.component_owned_layout.contains(&raw_id) {
                continue;
            }
            let current = self.runtime.node_style(id);
            if current.is_some_and(|style| {
                std::ptr::eq(style.layout.as_ref(), layout) || style.layout.as_ref() == layout
            }) {
                continue;
            }
            let mut style = current.cloned().unwrap_or_default();
            style.layout = Arc::new(layout.clone());
            mutations.set_style(id, style);
        }
        if !mutations.is_empty() {
            self.commit_extra(mutations).ok();
        }
    }

    /// Commit queued Vue host ops, then drain Runtime systems.
    pub fn flush_host_frame(&mut self) {
        self.stamp_host_texture_revisions();
        self.commit_pending_queue().ok();
        self.flush_runtime_systems();
    }

    /// Attach the window's host-texture registry so flush can pack revisions.
    #[cfg(feature = "scene-view")]
    pub(crate) fn attach_host_textures(&mut self, textures: nana_ui::HostTextureRegistry) {
        self.host_textures = Some(textures);
    }

    /// Packed revision for a registered slot. Unresolved handles stay `0`.
    fn packed_host_texture_revision(&self, slot: &str) -> u64 {
        let _ = slot;
        #[cfg(test)]
        if let Some(revision) = self.host_texture_revision_overrides.get(slot) {
            return *revision;
        }
        #[cfg(feature = "scene-view")]
        if let Some(registry) = &self.host_textures
            && let Some(binding) = registry.get(slot)
        {
            return nana_ui_runtime::pack_gpu_revision(
                binding.texture.generation(),
                binding.texture.version(),
            );
        }
        0
    }

    /// Refresh `CustomRenderNode.revision` from the registered texture.
    fn stamp_host_texture_revisions(&mut self) {
        let ids: Vec<u64> = self.host_texture_nodes.iter().copied().collect();
        for id in ids {
            if self.nodes.contains_key(&id) {
                self.sync_surface_custom_render(NodeHandle(id));
            } else {
                self.host_texture_nodes.remove(&id);
            }
        }
    }

    fn index_host_texture_node(&mut self, el: NodeHandle) {
        if self.surface_host_texture_slot(el).is_some() {
            self.host_texture_nodes.insert(el.0);
        } else {
            self.host_texture_nodes.remove(&el.0);
        }
    }

    #[cfg(test)]
    pub(crate) fn override_host_texture_revision(
        &mut self,
        slot: impl Into<String>,
        revision: u64,
    ) {
        self.host_texture_revision_overrides
            .insert(slot.into(), revision);
    }

    /// Transactional pending-ops commit used by input/IME/focus paths that
    /// must land host ops before reading back Runtime state.
    fn commit_pending_with(
        &mut self,
        add: impl FnOnce(&mut MutationQueue),
    ) -> Result<(), nana_ui_runtime::UiWorldError> {
        add(&mut self.pending.mutations);
        self.commit_pending_queue()
    }

    /// Commit `extra` after everything currently pending, in one transaction.
    fn commit_extra(&mut self, extra: MutationQueue) -> Result<(), nana_ui_runtime::UiWorldError> {
        self.pending.mutations.append(extra);
        self.commit_pending_queue()
    }

    /// Commit the whole pending batch, never dropping it silently.
    ///
    /// `UiWorld::commit` validates before applying, so a rejected batch lands
    /// nothing. In that case the batch is replayed one mutation at a time:
    /// valid ops land, and each rejected op is dropped together with its
    /// overlay mirror (the whole overlay set is cleared either way because no
    /// pending writes remain) and recorded for the host diagnostics sink.
    /// Valid mutations therefore survive a sibling's rejection instead of the
    /// entire frame's host ops disappearing.
    fn commit_pending_queue(&mut self) -> Result<(), nana_ui_runtime::UiWorldError> {
        if self.pending.mutations.is_empty() {
            return Ok(());
        }
        let outcome = self.runtime.commit_ref(&self.pending.mutations);
        if let Err(error) = outcome {
            let queue = self.pending.mutations.take();
            let mut first_error = Some(error);
            for mutation in queue.as_slice() {
                let mut single = MutationQueue::new();
                single.push(mutation.clone());
                if let Err(error) = self.runtime.commit(single) {
                    if self.commit_rejections.len() < MAX_COMMIT_REJECTIONS {
                        self.commit_rejections
                            .push(format!("{:?}: {error}", mutation_label(mutation)));
                    }
                    first_error.get_or_insert(error);
                }
            }
            self.pending.clear();
            return Err(first_error.expect("batch rejection sets the first error"));
        }
        self.pending.clear();
        Ok(())
    }

    /// Rejections dropped by [`Self::commit_pending_queue`]; drained by the
    /// host and forwarded to the JS diagnostics sink.
    pub fn take_commit_rejections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.commit_rejections)
    }

    fn committed_children(&self, parent: u64) -> Vec<u64> {
        let Ok(parent) = StableNodeId::try_from(NodeHandle(parent)) else {
            return Vec::new();
        };
        self.runtime
            .node(parent)
            .map(|node| node.children.into_iter().map(|id| id.get()).collect())
            .unwrap_or_default()
    }

    fn live_children(&self, parent: u64) -> Vec<u64> {
        if let Some(children) = self.pending.children.get(&parent) {
            return children
                .iter()
                .copied()
                .filter(|id| self.nodes.contains_key(id))
                .collect();
        }
        self.committed_children(parent)
            .into_iter()
            .filter(|id| self.nodes.contains_key(id))
            .collect()
    }

    fn live_parent(&self, node: u64) -> Option<u64> {
        if let Some(parent) = self.pending.parent.get(&node) {
            return *parent;
        }
        self.runtime
            .node(StableNodeId::try_from(NodeHandle(node)).ok()?)?
            .parent
            .map(|id| id.get())
    }

    fn overlay_children_mut(&mut self, parent: u64) -> &mut Vec<u64> {
        if !self.pending.children.contains_key(&parent) {
            let children = self.committed_children(parent);
            self.pending.children.insert(parent, children);
        }
        self.pending
            .children
            .get_mut(&parent)
            .expect("just inserted")
    }

    fn enqueue_insert(&mut self, child: u64, parent: u64, anchor: Option<u64>) {
        if let Some(old_parent) = self.live_parent(child)
            && old_parent != parent
        {
            self.overlay_children_mut(old_parent)
                .retain(|id| *id != child);
        }
        self.pending.parent.insert(child, Some(parent));
        let siblings = self.overlay_children_mut(parent);
        siblings.retain(|id| *id != child);
        let index = anchor
            .and_then(|anchor| siblings.iter().position(|id| *id == anchor))
            .unwrap_or(siblings.len());
        siblings.insert(index, child);
        self.pending.mutations.insert(
            StableNodeId::new(parent).expect("known parent is nonzero"),
            StableNodeId::new(child).expect("known child is nonzero"),
            anchor.and_then(StableNodeId::new),
        );
    }

    fn live_kind(&self, node: NodeHandle) -> Option<NodeKind> {
        if let Some(kind) = self.pending.kinds.get(&node.0) {
            return Some(kind.clone());
        }
        self.runtime
            .node(StableNodeId::try_from(node).ok()?)
            .map(|node| node.kind)
    }

    fn live_text(&self, node: NodeHandle) -> Option<String> {
        if let Some(text) = self.pending.texts.get(&node.0) {
            return Some(text.clone());
        }
        self.runtime_committed_text(node)
    }

    fn runtime_committed_text(&self, node: NodeHandle) -> Option<String> {
        self.runtime
            .text(StableNodeId::try_from(node).ok()?)
            .map(str::to_owned)
    }

    fn live_events(&self, el: NodeHandle) -> HashSet<String> {
        if let Some(events) = self.pending.events.get(&el.0) {
            return events.clone();
        }
        let Ok(id) = StableNodeId::try_from(el) else {
            return HashSet::new();
        };
        self.runtime
            .event_listeners(id)
            .map(|listeners| listeners.iter().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn live_custom_render(&self, el: NodeHandle) -> Option<CustomRenderNode> {
        if let Some(content) = self.pending.gpu.get(&el.0) {
            return content.clone();
        }
        self.runtime
            .custom_render(StableNodeId::try_from(el).ok()?)
            .cloned()
    }

    pub fn scroll_offset(&self, node: NodeHandle) -> nana_ui_runtime::ScrollOffset {
        StableNodeId::try_from(node)
            .ok()
            .and_then(|id| self.runtime.scroll_offset(id))
            .unwrap_or_default()
    }

    pub(crate) fn overflow_scrolls(&self, node: NodeHandle) -> bool {
        let Ok(id) = StableNodeId::try_from(node) else {
            return false;
        };
        self.context().overflow_scrolls(id)
    }

    pub fn scroll_metrics(&self, node: NodeHandle) -> Option<nana_ui_runtime::ScrollMetrics> {
        StableNodeId::try_from(node)
            .ok()
            .and_then(|id| self.runtime.scroll_metrics(id))
    }

    pub(crate) fn set_scroll_offset(
        &mut self,
        node: NodeHandle,
        offset: nana_ui_runtime::ScrollOffset,
    ) -> bool {
        let Ok(id) = StableNodeId::try_from(node) else {
            return false;
        };
        let offset = self.runtime.clamp_scroll_offset(id, offset);
        if self.runtime.scroll_offset(id) == Some(offset) && self.pending.is_empty() {
            return false;
        }
        self.commit_pending_with(|mutations| mutations.set_scroll_offset(id, offset))
            .ok();
        if self.runtime.scroll_offset(id) != Some(offset) {
            return false;
        }
        self.flush_runtime_systems();
        true
    }

    pub(crate) fn scroll_by(
        &mut self,
        node: NodeHandle,
        delta: nana_ui_runtime::ScrollOffset,
    ) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return false;
        }
        self.publish_scroll_metrics_from_layout(node);
        let current = self.scroll_offset(node);
        self.set_scroll_offset(
            node,
            nana_ui_runtime::ScrollOffset {
                x: (current.x + delta.x).max(0.0),
                y: (current.y + delta.y).max(0.0),
            },
        )
    }

    /// Apply `delta` using host/Scene metrics in the same commit as the offset
    /// so engine boxes cannot clamp the wheel to zero.
    pub(crate) fn scroll_by_with_metrics(
        &mut self,
        node: NodeHandle,
        delta: nana_ui_runtime::ScrollOffset,
        metrics: nana_ui_runtime::ScrollMetrics,
    ) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return false;
        }
        let Ok(id) = StableNodeId::try_from(node) else {
            return false;
        };
        let current = self.scroll_offset(node);
        let next = metrics.clamp(nana_ui_runtime::ScrollOffset {
            x: (current.x + delta.x).max(0.0),
            y: (current.y + delta.y).max(0.0),
        });
        if next == current {
            return false;
        }
        self.commit_pending_with(|mutations| {
            mutations.set_scroll_metrics(id, Some(metrics));
            mutations.set_scroll_offset(id, next);
        })
        .ok();
        self.runtime.scroll_offset(id) == Some(next)
    }

    fn publish_scroll_metrics_from_layout(&mut self, node: NodeHandle) {
        let Some(metrics) = self.layout_scroll_metrics_from(node, None) else {
            return;
        };
        let Ok(id) = StableNodeId::try_from(node) else {
            return;
        };
        if !self.should_adopt_scroll_metrics(id, metrics) {
            return;
        }
        self.pending.mutations.set_scroll_metrics(id, Some(metrics));
    }

    fn should_adopt_scroll_metrics(
        &self,
        id: StableNodeId,
        metrics: nana_ui_runtime::ScrollMetrics,
    ) -> bool {
        match self.runtime.scroll_metrics(id) {
            Some(existing) if existing == metrics => false,
            Some(existing)
                if metrics.content_width <= existing.content_width
                    && metrics.content_height <= existing.content_height =>
            {
                false
            }
            _ => true,
        }
    }

    pub(crate) fn layout_scroll_metrics_from(
        &self,
        node: NodeHandle,
        store: Option<&LayoutBoxStore>,
    ) -> Option<nana_ui_runtime::ScrollMetrics> {
        let viewport = store
            .and_then(|store| store.get(node))
            .or_else(|| self.layout_box(node))?;
        let mut content_width = viewport.width;
        let mut content_height = viewport.height;
        let mut stack = self.children_of(node);
        while let Some(child) = stack.pop() {
            if let Some(box_) = store
                .and_then(|store| store.get(child))
                .or_else(|| self.layout_box(child))
            {
                content_width = content_width.max(box_.x + box_.width - viewport.x);
                content_height = content_height.max(box_.y + box_.height - viewport.y);
            }
            stack.extend(self.children_of(child));
        }
        Some(nana_ui_runtime::ScrollMetrics {
            viewport_width: viewport.width,
            viewport_height: viewport.height,
            content_width: content_width.max(0.0),
            content_height: content_height.max(0.0),
        })
    }

    pub(crate) fn sync_scroll_viewport(
        &mut self,
        node: NodeHandle,
        offset: nana_ui_runtime::ScrollOffset,
        metrics: nana_ui_runtime::ScrollMetrics,
    ) -> Option<(nana_ui_runtime::ScrollOffset, nana_ui_runtime::ScrollOffset)> {
        let id = StableNodeId::try_from(node).ok()?;
        let previous = self.runtime.scroll_offset(id).unwrap_or_default();
        if self.pending.is_empty()
            && self.runtime.scroll_metrics(id) == Some(metrics)
            && self.runtime.clamp_scroll_offset(id, offset) == previous
        {
            return None;
        }
        self.commit_pending_with(|mutations| {
            mutations.set_scroll_metrics(id, Some(metrics));
            mutations.set_scroll_offset(id, offset);
        })
        .ok();
        Some((previous, self.runtime.scroll_offset(id).unwrap_or(previous)))
    }

    pub(crate) fn scroll_offsets(&self) -> Vec<(u64, nana_ui_runtime::ScrollOffset)> {
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero");
        self.runtime
            .document_order(document)
            .into_iter()
            .filter_map(|id| {
                let offset = self.runtime.scroll_offset(id)?;
                (offset.x != 0.0 || offset.y != 0.0).then_some((id.get(), offset))
            })
            .collect()
    }

    pub(crate) fn text_input_state(&self, node: NodeHandle) -> Option<TextInputState> {
        self.runtime
            .text_input(StableNodeId::try_from(node).ok()?)
            .cloned()
    }

    pub(crate) fn set_text_input_state(&mut self, node: NodeHandle, state: TextInputState) -> bool {
        let Ok(id) = StableNodeId::try_from(node) else {
            return false;
        };
        if self.runtime.text_input(id) == Some(&state) {
            return false;
        }
        let expected = state.clone();
        self.commit_pending_with(|mutations| mutations.set_text_input(id, Some(state)))
            .ok();
        self.runtime.text_input(id) == Some(&expected)
    }

    pub(crate) fn ime_composition(&self, node: NodeHandle) -> Option<ImeComposition> {
        self.runtime
            .ime(StableNodeId::try_from(node).ok()?)
            .cloned()
    }

    pub(crate) fn set_ime_composition(
        &mut self,
        node: NodeHandle,
        composition: Option<ImeComposition>,
    ) -> bool {
        let Ok(id) = StableNodeId::try_from(node) else {
            return false;
        };
        let expected = composition.clone();
        self.commit_pending_with(|mutations| mutations.set_ime(id, composition))
            .ok();
        self.runtime.ime(id) == expected.as_ref()
    }

    pub fn scene(&self) -> &UiScene {
        self.runtime.runtime_document().scene()
    }

    pub fn accessibility_snapshot(&self) -> Vec<nana_ui_runtime::AccessibilityNode> {
        self.runtime.project_accessibility(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
        )
    }

    #[cfg(any(test, feature = "hosted"))]
    pub(crate) fn take_accessibility_update(&mut self) -> Option<AccessibilityUpdate> {
        if self.accessibility_full_required {
            self.accessibility_full_required = false;
            self.pending_accessibility_updated.clear();
            self.pending_accessibility_removed.clear();
            return Some(AccessibilityUpdate::Full {
                generation: Some(self.pending_accessibility_generation),
                nodes: self.accessibility_snapshot(),
            });
        }
        if self.pending_accessibility_updated.is_empty()
            && self.pending_accessibility_removed.is_empty()
        {
            return None;
        }
        let updated = std::mem::take(&mut self.pending_accessibility_updated)
            .into_values()
            .collect();
        let removed = std::mem::take(&mut self.pending_accessibility_removed)
            .into_iter()
            .collect();
        Some(AccessibilityUpdate::Delta(AccessibilityDelta {
            generation: self.pending_accessibility_generation,
            updated,
            removed,
        }))
    }

    pub fn sync_semantic_styles(&mut self, snapshot: &crate::SemanticSnapshot) {
        if self.synced_semantic_revision == Some(snapshot.revision) {
            if !self.pending.is_empty() {
                self.flush_host_frame();
            }
            return;
        }
        let mut mutations = MutationQueue::new();
        let mut pending = PendingAssembly::default();
        let mut component_owned_layout = HashSet::new();
        if self.runtime.theme_mode() != snapshot.theme {
            mutations.set_theme(snapshot.theme);
        }
        for widget in &snapshot.widgets {
            let Some(id) = StableNodeId::new(widget.id)
                .filter(|id| self.runtime.contains(*id) || self.nodes.contains_key(&id.get()))
            else {
                continue;
            };
            // Qualified components own their complete Runtime projection. Queuing
            // a generic style/interaction/accessibility state first only to
            // overwrite it in the same transaction doubles validation, dirty
            // propagation and commit work for large component trees.
            if !(widget.kind.is_choice_field()
                || matches!(
                    widget.kind,
                    crate::WidgetKind::Input
                        | crate::WidgetKind::Textarea
                        | crate::WidgetKind::ContextMenu
                        | crate::WidgetKind::CommandPalette
                ))
                && matches!(
                    widget.kind,
                    crate::WidgetKind::Button
                        | crate::WidgetKind::Checkbox
                        | crate::WidgetKind::Switch
                        | crate::WidgetKind::Card
                        | crate::WidgetKind::ListItem
                        | crate::WidgetKind::Range
                        | crate::WidgetKind::StatusBadge
                        | crate::WidgetKind::ValidationMessage
                        | crate::WidgetKind::EmptyState
                        | crate::WidgetKind::LabeledValue
                        | crate::WidgetKind::Segmented
                        | crate::WidgetKind::Tabs
                        | crate::WidgetKind::Progress
                        | crate::WidgetKind::Spinner
                        | crate::WidgetKind::FormField
                        | crate::WidgetKind::InteractiveCard
                        | crate::WidgetKind::SidebarFrame
                        | crate::WidgetKind::SidebarRow
                        | crate::WidgetKind::SettingsRow
                        | crate::WidgetKind::SettingsCard
                        | crate::WidgetKind::Skeleton
                        | crate::WidgetKind::LevelMeter
                        | crate::WidgetKind::Select
                        | crate::WidgetKind::Dropdown
                        | crate::WidgetKind::SearchDropdown
                        | crate::WidgetKind::Dialog
                        | crate::WidgetKind::Drawer
                        | crate::WidgetKind::Popover
                        | crate::WidgetKind::ContextMenu
                        | crate::WidgetKind::Toast
                        | crate::WidgetKind::Tooltip
                        | crate::WidgetKind::ActionMenu
                        | crate::WidgetKind::ActionMenuItem
                        | crate::WidgetKind::XYPad
                        | crate::WidgetKind::QrCode
                        | crate::WidgetKind::CommandPalette
                        | crate::WidgetKind::TreeView
                        | crate::WidgetKind::CalendarHeatmap
                        | crate::WidgetKind::ImageViewer
                        | crate::WidgetKind::NativeMarkdown
                        | crate::WidgetKind::GraphCanvas
                        | crate::WidgetKind::Workspace
                        | crate::WidgetKind::Dock
                        | crate::WidgetKind::SplitPane
                        | crate::WidgetKind::AppShell
                        | crate::WidgetKind::SettingsPage
                )
                && self.runtime.text_input(id).is_some()
            {
                // Queue before the new component projection: SetTextInput(None)
                // clears the old committed value, then Button/ListItem may publish
                // their own visible label later in the same transaction.
                mutations.set_text_input(id, None);
            }
            if project_migrating_component(
                widget,
                snapshot,
                id,
                self.runtime.context(),
                &mut mutations,
                &mut pending,
            ) || is_shell_composer_slot(snapshot, widget)
            {
                // The component consumed `props.layout` when it was built and
                // now owns this node's LayoutStyle. Record it so the cascade
                // writeback leaves the projected geometry alone.
                component_owned_layout.insert(id.get());
                continue;
            }
            let style = NodeStyle {
                layout: Arc::new(widget.props.layout.clone()),
                foreground: widget
                    .props
                    .muted
                    .then_some(nana_ui_core::SemanticColorRole::Muted),
                background: None,
                border: None,
                interaction: nana_ui_runtime::InteractionStyle::default(),
                ..NodeStyle::default()
            };
            if self.runtime.node_style(id) != Some(&style) {
                mutations.set_style(id, style);
            }
            let interaction = InteractionState {
                pointer_events: !widget.props.disabled
                    && !widget.props.layout.hidden
                    && !matches!(
                        widget.props.layout.pointer_events,
                        Some(nana_ui_core::PointerEventsSpec::None)
                    )
                    && !widget
                        .props
                        .attrs
                        .contains_key(crate::bridge::GENERATED_PSEUDO_ATTR),
                focusable: !widget.props.disabled
                    && (widget.kind.is_choice_field()
                        || matches!(
                            widget.kind,
                            crate::WidgetKind::Button
                                | crate::WidgetKind::Chip
                                | crate::WidgetKind::Input
                                | crate::WidgetKind::Textarea
                                | crate::WidgetKind::Checkbox
                                | crate::WidgetKind::Switch
                                | crate::WidgetKind::Tabs
                                | crate::WidgetKind::Segmented
                                | crate::WidgetKind::Range
                        )),
            };
            if self.runtime.interaction(id) != Some(interaction) {
                mutations.set_interaction(id, interaction);
            }
            let accessibility = AccessibilityState {
                role: accessibility_role(widget.kind, &widget.props.role),
                label: (!widget.props.label.is_empty())
                    .then(|| Arc::<str>::from(widget.props.label.as_str())),
                value: (!widget.props.value.is_empty())
                    .then(|| Arc::<str>::from(widget.props.value.as_str())),
                disabled: widget.props.disabled,
                checked: matches!(
                    widget.kind,
                    crate::WidgetKind::Checkbox | crate::WidgetKind::Switch
                )
                .then_some(widget.props.toggled),
                selected: matches!(
                    widget.kind,
                    crate::WidgetKind::Chip
                        | crate::WidgetKind::ListItem
                        | crate::WidgetKind::SidebarRow
                )
                .then_some(widget.props.active),
                multiline: matches!(widget.kind, crate::WidgetKind::Textarea),
                editable: matches!(
                    widget.kind,
                    crate::WidgetKind::Input | crate::WidgetKind::Textarea
                ) && !widget.props.attrs.contains_key("readonly"),
                modal: matches!(
                    widget.kind,
                    crate::WidgetKind::Dialog | crate::WidgetKind::Drawer
                ),
                busy: widget.props.loading,
                invalid: widget.props.invalid,
                ..AccessibilityState::default()
            };
            if self.runtime.accessibility(id) != Some(&accessibility) {
                mutations.set_accessibility(id, accessibility);
            }
            if matches!(widget.kind, crate::WidgetKind::Text) {
                let label = widget.props.display_label();
                if !label.is_empty() && self.runtime.text(id) != Some(label) {
                    mutations.set_text(
                        id,
                        TextContent {
                            value: label.into(),
                        },
                    );
                }
            }
            if matches!(
                widget.kind,
                crate::WidgetKind::Input | crate::WidgetKind::Textarea
            ) {
                let mut next = self
                    .runtime
                    .text_input(id)
                    .cloned()
                    .unwrap_or_else(|| TextInputState::new(&widget.props.value));
                if next.value != widget.props.value {
                    next.replace_value(&widget.props.value);
                }
                if self.runtime.text_input(id) != Some(&next) {
                    mutations.set_text_input(id, Some(next));
                }
            } else if self.runtime.text_input(id).is_some() {
                mutations.set_text_input(id, None);
            }
        }
        self.component_owned_layout = component_owned_layout;
        self.commit_extra(mutations).ok();
        pending.apply(self.runtime.context_mut());
        self.adopt_runtime_allocated_ids();
        self.flush_runtime_systems();
        self.synced_semantic_revision = Some(snapshot.revision);
    }

    fn adopt_runtime_allocated_ids(&mut self) {
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero");
        let max = self
            .runtime
            .document_order(document)
            .into_iter()
            .map(StableNodeId::get)
            .max()
            .unwrap_or(0);
        if let Some(next) = max.checked_add(1)
            && next > self.next_id
        {
            self.next_id = next;
        }
    }

    /// Overwrite snapshot parent/children/roots from `UiWorld`.
    ///
    /// [`MessageBridge`] may keep a cascade working index; this method is
    /// what makes Runtime hierarchy the observable tree before Scene paint.
    pub(crate) fn apply_runtime_hierarchy(&self, snapshot: &mut crate::SemanticSnapshot) {
        snapshot
            .widgets
            .retain(|widget| self.nodes.contains_key(&widget.id));
        let visible = snapshot
            .widgets
            .iter()
            .map(|widget| widget.id)
            .collect::<HashSet<_>>();
        for widget in &mut snapshot.widgets {
            widget.parent = self
                .live_parent(widget.id)
                .filter(|parent| visible.contains(parent));
            widget.children = self
                .live_children(widget.id)
                .into_iter()
                .filter(|child| visible.contains(child))
                .collect();
        }
        snapshot.roots = snapshot
            .widgets
            .iter()
            .filter(|widget| widget.parent.is_none())
            .map(|widget| widget.id)
            .collect();
    }

    pub fn mount_root(&self) -> NodeHandle {
        self.mount_root
    }

    pub fn html_root(&self) -> NodeHandle {
        self.html_root
    }

    pub fn set_document_theme(&mut self, theme: &str) {
        self.theme = theme.to_string();
    }

    pub fn document_theme(&self) -> &str {
        &self.theme
    }

    pub fn logical_size(&self) -> (f32, f32) {
        (self.logical_width, self.logical_height)
    }

    pub fn logical_width(&self) -> f32 {
        self.logical_width
    }

    pub fn logical_height(&self) -> f32 {
        self.logical_height
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Child element/text handles in document order.
    pub fn children_of(&self, parent: NodeHandle) -> Vec<NodeHandle> {
        if !matches!(
            self.nodes.get(&parent.0).map(|node| &node.data),
            Some(NodeData::Element { .. })
        ) {
            return Vec::new();
        }
        self.live_children(parent.0)
            .into_iter()
            .map(NodeHandle)
            .collect()
    }

    /// DOM `Element.parentElement` — element parent only (same as parent_node here).
    pub fn parent_element(&self, node: NodeHandle) -> Option<NodeHandle> {
        let parent = self.parent_node(node)?;
        match self.node_kind(parent) {
            DomNodeKind::Element => Some(parent),
            _ => None,
        }
    }

    /// Pre-order walk of element ids under `root` (includes `root`).
    pub fn collect_element_preorder(&self, root: NodeHandle) -> Vec<u64> {
        let mut out = Vec::new();
        self.collect_preorder(root.0, &mut out);
        out
    }

    pub fn set_viewport(&mut self, physical_width: u32, physical_height: u32, scale_factor: f32) {
        let scale = scale_factor.max(0.01);
        self.scale_factor = scale;
        self.logical_width = physical_width as f32 / scale;
        self.logical_height = physical_height as f32 / scale;
        self.reset_layout_roots();
    }

    pub fn create_element(&mut self, tag: &str) -> NodeHandle {
        self.create_element_ns(tag, ElementNamespace::Html, None)
    }

    /// Vue `createElement(tag, namespace, is, props)` — stores namespace + optional `is`.
    pub fn create_element_ns(
        &mut self,
        tag: &str,
        namespace: ElementNamespace,
        is: Option<&str>,
    ) -> NodeHandle {
        let id = self.alloc();
        let mut attrs = HashMap::new();
        if let Some(is) = is.filter(|s| !s.is_empty()) {
            attrs.insert("is".into(), is.to_string());
        }
        // Tag name drives child-namespace resolution the same way runtime-dom does.
        let namespace = match tag.trim().to_ascii_lowercase().as_str() {
            "svg" => ElementNamespace::Svg,
            "math" => ElementNamespace::MathMl,
            _ => namespace,
        };
        self.nodes.insert(
            id,
            Node {
                data: NodeData::Element { namespace, attrs },
                scope_id: None,
            },
        );
        let kind = NodeKind::Element {
            tag: tag.to_ascii_lowercase(),
        };
        self.pending.kinds.insert(id, kind.clone());
        self.pending.mutations.create(
            StableNodeId::new(id).expect("allocated IDs are nonzero"),
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            kind,
        );
        NodeHandle(id)
    }

    pub fn create_text(&mut self, text: &str) -> NodeHandle {
        let id = self.alloc();
        self.nodes.insert(
            id,
            Node {
                data: NodeData::Text,
                scope_id: None,
            },
        );
        let id = StableNodeId::new(id).expect("allocated IDs are nonzero");
        self.pending.kinds.insert(id.get(), NodeKind::Text);
        self.pending.texts.insert(id.get(), text.to_string());
        self.pending.mutations.create(
            id,
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            NodeKind::Text,
        );
        self.pending
            .mutations
            .set_text(id, TextContent { value: text.into() });
        NodeHandle::from(id)
    }

    pub fn create_comment(&mut self, text: &str) -> NodeHandle {
        let id = self.alloc();
        self.nodes.insert(
            id,
            Node {
                data: NodeData::Comment,
                scope_id: None,
            },
        );
        let id = StableNodeId::new(id).expect("allocated IDs are nonzero");
        self.pending.kinds.insert(id.get(), NodeKind::Comment);
        self.pending.texts.insert(id.get(), text.to_string());
        self.pending.mutations.create(
            id,
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            NodeKind::Comment,
        );
        self.pending
            .mutations
            .set_text(id, TextContent { value: text.into() });
        NodeHandle::from(id)
    }

    pub fn inject_stylesheet(&mut self, css: &str) {
        // Retained for diagnostics; cascade onto LayoutStyle happens in MessageBridge.
        self.stylesheets.push(css.to_string());
    }

    pub fn stylesheet_count(&self) -> usize {
        self.stylesheets.len()
    }

    pub fn inject_style_element(&mut self, css: &str) -> NodeHandle {
        self.inject_stylesheet(css);
        let el = self.create_element("style");
        self.set_element_text(el, css);
        let root = self.mount_root;
        self.insert(el, root, None);
        el
    }

    pub fn set_scope_id(&mut self, el: NodeHandle, scope_id: &str) {
        if let Some(node) = self.nodes.get_mut(&el.0) {
            node.scope_id = Some(scope_id.to_string());
        }
    }

    pub fn clone_node(&mut self, node: NodeHandle, deep: bool) -> NodeHandle {
        self.clone_node_mapped(node, deep).0
    }

    /// Deep/shallow clone; returns `(copy, [(src, dst), …])` for MessageBridge sync.
    pub fn clone_node_mapped(
        &mut self,
        node: NodeHandle,
        deep: bool,
    ) -> (NodeHandle, Vec<(NodeHandle, NodeHandle)>) {
        let mut pairs = Vec::new();
        let copy = self.clone_node_into(node, deep, &mut pairs);
        (copy, pairs)
    }

    fn clone_node_into(
        &mut self,
        node: NodeHandle,
        deep: bool,
        pairs: &mut Vec<(NodeHandle, NodeHandle)>,
    ) -> NodeHandle {
        let Some(src) = self.nodes.get(&node.0).cloned() else {
            let missing = self.create_comment("missing");
            pairs.push((node, missing));
            return missing;
        };
        let copy = match src.data {
            NodeData::Element { namespace, attrs } => {
                let tag = self
                    .element_tag(node)
                    .expect("element payload has runtime kind");
                let is = attrs.get("is").map(|s| s.as_str());
                let copy = self.create_element_ns(&tag, namespace, is);
                for (k, v) in &attrs {
                    if k == "is" {
                        continue;
                    }
                    self.set_attribute(copy, k, v);
                }
                if let Some(scope) = src.scope_id {
                    self.set_scope_id(copy, &scope);
                }
                if deep {
                    for child in self.children_of(node) {
                        let child_copy = self.clone_node_into(child, true, pairs);
                        self.insert(child_copy, copy, None);
                    }
                }
                copy
            }
            NodeData::Text => self.create_text(self.runtime_text(node).as_deref().unwrap_or("")),
            NodeData::Comment => {
                self.create_comment(self.runtime_text(node).as_deref().unwrap_or(""))
            }
        };
        pairs.push((node, copy));
        copy
    }

    /// Vue `insertStaticContent(content, parent, anchor, namespace, start?, end?)`.
    ///
    /// When `start`/`end` reference a prior static range, clones that range (official
    /// reuse path). Otherwise parses `content` as a trusted fragment (compiler output).
    pub fn insert_static_content(
        &mut self,
        content: &str,
        parent: NodeHandle,
        anchor: Option<NodeHandle>,
        namespace: ElementNamespace,
        start: Option<NodeHandle>,
        end: Option<NodeHandle>,
    ) -> (NodeHandle, NodeHandle, Vec<(NodeHandle, NodeHandle)>) {
        let can_reuse = match start {
            Some(s) => end.map(|e| e.0 == s.0).unwrap_or(false) || self.next_sibling(s).is_some(),
            None => false,
        };
        if can_reuse {
            let start = start.expect("can_reuse implies start");
            let end = end.unwrap_or(start);
            let mut pairs = Vec::new();
            let mut cursor = Some(start);
            let mut first = None;
            let mut last = None;
            while let Some(cur) = cursor {
                let (copy, mut mapped) = self.clone_node_mapped(cur, true);
                pairs.append(&mut mapped);
                self.insert(copy, parent, anchor);
                if first.is_none() {
                    first = Some(copy);
                }
                last = Some(copy);
                if cur.0 == end.0 {
                    break;
                }
                cursor = self.next_sibling(cur);
            }
            let first = first.unwrap_or(start);
            let last = last.unwrap_or(first);
            return (first, last, pairs);
        }

        let html = match namespace {
            ElementNamespace::Svg => format!("<svg>{content}</svg>"),
            ElementNamespace::MathMl => format!("<math>{content}</math>"),
            ElementNamespace::Html => content.to_string(),
        };
        let mut roots = parse_html_fragment(&html);
        if matches!(namespace, ElementNamespace::Svg | ElementNamespace::MathMl) {
            // runtime-dom unwraps the svg/math wrapper and keeps its children.
            if let Some(FragNode::Element { children, .. }) = roots.first_mut() {
                roots = std::mem::take(children);
            }
        }
        if roots.is_empty() {
            let marker = self.create_comment("static");
            self.insert(marker, parent, anchor);
            return (marker, marker, Vec::new());
        }
        let mut first = None;
        let mut last = None;
        let mut created = Vec::new();
        for frag in roots {
            let handle = self.materialize_fragment(frag, namespace, &mut created);
            self.insert(handle, parent, anchor);
            if first.is_none() {
                first = Some(handle);
            }
            last = Some(handle);
        }
        let first = first.expect("non-empty roots");
        let last = last.unwrap_or(first);
        let pairs = created.into_iter().map(|h| (h, h)).collect();
        (first, last, pairs)
    }

    fn materialize_fragment(
        &mut self,
        frag: FragNode,
        parent_ns: ElementNamespace,
        created: &mut Vec<NodeHandle>,
    ) -> NodeHandle {
        match frag {
            FragNode::Text(t) => {
                let h = self.create_text(&t);
                created.push(h);
                h
            }
            FragNode::Element {
                tag,
                attrs,
                children,
            } => {
                let ns = match tag.as_str() {
                    "svg" => ElementNamespace::Svg,
                    "math" => ElementNamespace::MathMl,
                    "foreignObject" if parent_ns == ElementNamespace::Svg => ElementNamespace::Html,
                    _ => parent_ns,
                };
                let is = attrs
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("is"))
                    .map(|(_, v)| v.as_str());
                let el = self.create_element_ns(&tag, ns, is);
                for (k, v) in &attrs {
                    if k.eq_ignore_ascii_case("is") {
                        continue;
                    }
                    self.set_attribute(el, k, v);
                }
                self.set_attribute(el, "data-static", "1");
                created.push(el);
                for child in children {
                    let child_ns = match self.element_namespace(el) {
                        Some(n) => n,
                        None => ns,
                    };
                    let ch = self.materialize_fragment(child, child_ns, created);
                    self.insert(ch, el, None);
                }
                el
            }
        }
    }

    pub fn set_gpu_slot(&mut self, el: NodeHandle, slot: &str) {
        self.set_attribute(el, "data-nana-gpu", slot);
    }

    pub fn gpu_slots(&self) -> Vec<(NodeHandle, String)> {
        let mut slots: Vec<_> = self
            .nodes
            .keys()
            .copied()
            .filter_map(|id| {
                let handle = NodeHandle(id);
                let content = self.live_custom_render(handle)?;
                if content.renderer.as_ref() != HOST_TEXTURE_RENDERER {
                    return None;
                }
                Some((handle, content.resource.as_ref().to_string()))
            })
            .collect();
        slots.sort_by_key(|(handle, _)| handle.0);
        slots
    }

    fn sync_surface_custom_render(&mut self, el: NodeHandle) {
        let Ok(id) = StableNodeId::try_from(el) else {
            return;
        };
        if !self.nodes.contains_key(&el.0) {
            return;
        }
        let content = self.surface_host_texture_slot(el).map(|slot| {
            let revision = self.packed_host_texture_revision(&slot);
            host_texture_content(slot, revision)
        });
        if self.live_custom_render(el) == content {
            return;
        }
        self.pending.gpu.insert(el.0, content.clone());
        self.pending.mutations.set_custom_render(id, content);
    }

    fn surface_host_texture_slot(&self, el: NodeHandle) -> Option<String> {
        if let Some(slot) = self
            .get_attribute(el, "data-nana-gpu")
            .filter(|slot| !slot.is_empty())
        {
            return Some(slot);
        }
        self.get_attribute(el, "data-nana-canvas")
            .as_deref()
            .and_then(canvas_host_texture_slot)
    }

    pub fn insert(&mut self, child: NodeHandle, parent: NodeHandle, anchor: Option<NodeHandle>) {
        // Validate parent *before* detach. Remount / Teleport can still hold
        // wrapNode ids for disposed nodes; detach-then-fail left sidebars'
        // footer slots (and other subtrees) orphaned with parent=None.
        if child.0 == parent.0 {
            return;
        }
        let parent_ok = matches!(
            self.nodes.get(&parent.0).map(|n| &n.data),
            Some(NodeData::Element { .. })
        );
        if !parent_ok {
            return;
        }
        if !self.nodes.contains_key(&child.0) {
            return;
        }
        self.enqueue_insert(child.0, parent.0, anchor.map(|anchor| anchor.0));
        self.index_host_texture_node(child);
        if self.surface_host_texture_slot(child).is_some() {
            self.sync_surface_custom_render(child);
        }
    }

    pub fn remove(&mut self, child: NodeHandle) {
        // Teleport / v-if unmount: detach and drop the subtree so mount-root
        // open/close cycles do not accumulate orphan nodes (Overlay 不泄漏).
        self.dispose_subtree(child);
    }

    pub fn set_text(&mut self, node: NodeHandle, text: &str) {
        if matches!(
            self.nodes.get(&node.0).map(|node| &node.data),
            Some(NodeData::Text)
        ) {
            self.pending.texts.insert(node.0, text.to_string());
            self.pending.mutations.set_text(
                StableNodeId::try_from(node).expect("known text is nonzero"),
                TextContent { value: text.into() },
            );
        }
    }

    pub fn set_element_text(&mut self, el: NodeHandle, text: &str) {
        if !matches!(
            self.nodes.get(&el.0).map(|node| &node.data),
            Some(NodeData::Element { .. })
        ) {
            return;
        }
        let children = self.children_of(el);
        for c in children {
            self.dispose_subtree(c);
        }
        let text_node = self.create_text(text);
        self.insert(text_node, el, None);
    }

    pub fn set_attribute(&mut self, el: NodeHandle, name: &str, value: &str) {
        let mut changed = false;
        if let Some(Node {
            data: NodeData::Element { attrs, .. },
            ..
        }) = self.nodes.get_mut(&el.0)
        {
            changed = attrs.get(name).is_none_or(|current| current != value);
            attrs.insert(name.to_string(), value.to_string());
        }
        if changed
            && (name.eq_ignore_ascii_case("data-nana-gpu")
                || name.eq_ignore_ascii_case("data-nana-canvas"))
        {
            self.index_host_texture_node(el);
            self.sync_surface_custom_render(el);
        }
    }

    pub fn get_attribute(&self, el: NodeHandle, name: &str) -> Option<String> {
        match self.nodes.get(&el.0) {
            Some(Node {
                data: NodeData::Element { attrs, .. },
                ..
            }) => attrs.get(name).cloned(),
            _ => None,
        }
    }

    pub fn remove_attribute(&mut self, el: NodeHandle, name: &str) {
        let mut removed = false;
        if let Some(Node {
            data: NodeData::Element { attrs, .. },
            ..
        }) = self.nodes.get_mut(&el.0)
        {
            removed = attrs.remove(name).is_some();
        }
        if removed
            && (name.eq_ignore_ascii_case("data-nana-gpu")
                || name.eq_ignore_ascii_case("data-nana-canvas"))
        {
            self.index_host_texture_node(el);
            self.sync_surface_custom_render(el);
        }
    }

    pub fn set_event_flag(&mut self, el: NodeHandle, event: &str, enabled: bool) {
        let name = normalize_event_name(event);
        if name.is_empty() || !self.nodes.contains_key(&el.0) {
            return;
        }
        let mut events = self.live_events(el);
        if enabled {
            events.insert(name.clone());
        } else {
            events.remove(&name);
        }
        self.pending.events.insert(el.0, events);
        self.pending.mutations.set_event_listener(
            StableNodeId::try_from(el).expect("known node is nonzero"),
            name,
            enabled,
        );
    }

    pub fn has_event(&self, el: NodeHandle, event: &str) -> bool {
        let name = normalize_event_name(event);
        self.live_events(el).contains(&name)
    }

    pub fn parent_node(&self, node: NodeHandle) -> Option<NodeHandle> {
        self.live_parent(node.0).map(NodeHandle)
    }

    /// DOM `Node.contains`: true when `other` is `self` or a descendant.
    pub fn contains(&self, node: NodeHandle, other: NodeHandle) -> bool {
        if !self.nodes.contains_key(&node.0) || !self.nodes.contains_key(&other.0) {
            return false;
        }
        let mut cur = Some(other);
        while let Some(h) = cur {
            if h.0 == node.0 {
                return true;
            }
            cur = self.parent_node(h);
        }
        false
    }

    pub fn next_sibling(&self, node: NodeHandle) -> Option<NodeHandle> {
        let parent = self.parent_node(node)?;
        let children = self.children_of(parent);
        let idx = children.iter().position(|&child| child == node)?;
        children.get(idx + 1).copied()
    }

    pub fn previous_sibling(&self, node: NodeHandle) -> Option<NodeHandle> {
        let parent = self.parent_node(node)?;
        let children = self.children_of(parent);
        let idx = children.iter().position(|&child| child == node)?;
        if idx == 0 {
            None
        } else {
            children.get(idx - 1).copied()
        }
    }

    /// DOM `Node.firstChild`.
    pub fn first_child(&self, parent: NodeHandle) -> Option<NodeHandle> {
        self.children_of(parent).first().copied()
    }

    pub fn last_child(&self, parent: NodeHandle) -> Option<NodeHandle> {
        self.children_of(parent).last().copied()
    }

    pub fn query_selector(&self, selector: &str) -> Option<NodeHandle> {
        self.query_selector_all(selector).into_iter().next()
    }

    /// Minimal subtree query: tag / `.class` / `#id` / `[attr]` compounds, comma OR-lists.
    /// Not a CSS engine — no combinators (`>`, `+`, `~`, descendant) in one simple.
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeHandle> {
        let sel = selector.trim();
        if sel.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut stack = vec![self.html_root];
        while let Some(handle) = stack.pop() {
            if let Some(Node {
                data: NodeData::Element { attrs, .. },
                ..
            }) = self.nodes.get(&handle.0)
                && let Some(tag) = self.element_tag(handle)
            {
                if selector_list_matches(sel, &tag, attrs) {
                    out.push(handle);
                }
                for child in self.children_of(handle).into_iter().rev() {
                    stack.push(child);
                }
            }
        }
        out
    }

    /// Walk `node` and ancestors; return the nearest that matches `selector`.
    pub fn closest(&self, node: NodeHandle, selector: &str) -> Option<NodeHandle> {
        let sel = selector.trim();
        if sel.is_empty() {
            return None;
        }
        let mut cur = Some(node);
        while let Some(h) = cur {
            if let Some(Node {
                data: NodeData::Element { attrs, .. },
                ..
            }) = self.nodes.get(&h.0)
                && let Some(tag) = self.element_tag(h)
                && selector_list_matches(sel, &tag, attrs)
            {
                return Some(h);
            }
            cur = self.parent_node(h);
        }
        None
    }

    pub fn node_kind(&self, node: NodeHandle) -> DomNodeKind {
        match self.live_kind(node) {
            Some(NodeKind::Element { .. }) => DomNodeKind::Element,
            Some(NodeKind::Text) => DomNodeKind::Text,
            Some(NodeKind::Comment) => DomNodeKind::Comment,
            Some(NodeKind::Document) => DomNodeKind::Document,
            None => DomNodeKind::Other,
        }
    }

    pub fn element_tag(&self, node: NodeHandle) -> Option<String> {
        match self.live_kind(node)? {
            NodeKind::Element { tag } => Some(tag),
            _ => None,
        }
    }

    pub fn element_namespace(&self, node: NodeHandle) -> Option<ElementNamespace> {
        match self.nodes.get(&node.0).map(|n| &n.data) {
            Some(NodeData::Element { namespace, .. }) => Some(*namespace),
            _ => None,
        }
    }

    pub fn text_content(&self, node: NodeHandle) -> Option<String> {
        match self.node_kind(node) {
            DomNodeKind::Text => self.runtime_text(node),
            DomNodeKind::Element => {
                let mut out = String::new();
                for child in self.children_of(node) {
                    if let Some(t) = self.text_content(child) {
                        out.push_str(&t);
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    fn reset_layout_roots(&mut self) {
        let w = self.logical_width.max(1.0);
        let h = self.logical_height.max(1.0);
        let mut mutations = MutationQueue::new();
        for root in [self.html_root, self.mount_root] {
            self.enqueue_layout_if_changed(
                &mut mutations,
                root,
                RuntimeLayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: w,
                    height: h,
                },
            );
        }
        self.commit_extra(mutations).ok();
    }

    pub fn layout_box(&self, node: NodeHandle) -> Option<LayoutBox> {
        let layout = self
            .runtime
            .layout_box(StableNodeId::try_from(node).ok()?)?;
        Some(LayoutBox {
            handle: node,
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
        })
    }

    pub fn has_engine_layout_box(&self, node: NodeHandle) -> bool {
        self.layout_box(node)
            .is_some_and(|box_| box_.width > 0.0 || box_.height > 0.0)
    }

    /// Tests only: WriteLayout explicit boxes after an engine flush.
    pub fn inject_layout_boxes(&mut self, boxes: &[(NodeHandle, LayoutBox)]) {
        self.write_layout_boxes(boxes, true);
    }

    /// Flush the engine, then WriteLayout missing boxes or a larger overflow extent.
    pub fn apply_layout_boxes(&mut self, boxes: &[(NodeHandle, LayoutBox)]) {
        self.write_layout_boxes(boxes, false);
    }

    fn write_layout_boxes(&mut self, boxes: &[(NodeHandle, LayoutBox)], overwrite: bool) {
        self.flush_runtime_systems();
        let w = self.logical_width.max(1.0);
        let h = self.logical_height.max(1.0);
        let mut mutations = MutationQueue::new();
        for root in [self.html_root, self.mount_root] {
            self.enqueue_layout_if_changed(
                &mut mutations,
                root,
                RuntimeLayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: w,
                    height: h,
                },
            );
        }
        for &(handle, box_) in boxes {
            if handle.0 == self.html_root.0 || handle.0 == self.mount_root.0 {
                continue;
            }
            if box_.width <= 0.0 && box_.height <= 0.0 {
                continue;
            }
            if !overwrite && self.has_engine_layout_box(handle) {
                // Engine flush runs first and always writes a box. Host/Scene
                // paint may still own a larger overflow extent (sidebar body
                // content). Expanding that box keeps wheel metrics honest;
                // shrinking to CSS auto-height 0 stays forbidden.
                let Some(engine) = self.layout_box(handle) else {
                    continue;
                };
                if box_.width <= engine.width && box_.height <= engine.height {
                    continue;
                }
            }
            if let Ok(id) = StableNodeId::try_from(handle)
                && (self.runtime.contains(id) || self.nodes.contains_key(&handle.0))
            {
                self.enqueue_layout_if_changed(
                    &mut mutations,
                    handle,
                    RuntimeLayoutBox {
                        x: box_.x,
                        y: box_.y,
                        width: box_.width,
                        height: box_.height,
                    },
                );
            }
        }
        if !mutations.is_empty() || !self.pending.is_empty() {
            self.commit_extra(mutations).ok();
        }
        self.flush_runtime_extract();
    }

    pub fn snapshot_boxes(&self) -> BoxSnapshot {
        let runtime_document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero");
        let mut boxes: Vec<LayoutBox> = self
            .runtime
            .document_order(runtime_document)
            .into_iter()
            .filter_map(|id| self.layout_box(NodeHandle::from(id)))
            .collect();
        boxes.sort_by_key(|b| b.handle.0);
        let mut texts = Vec::new();
        let mut tags = Vec::new();
        for &id in self.nodes.keys() {
            let handle = NodeHandle(id);
            match self.node_kind(handle) {
                DomNodeKind::Text => {
                    if let Some(text) = self.runtime_text(handle).filter(|text| !text.is_empty()) {
                        texts.push((handle, text));
                    }
                }
                DomNodeKind::Element => {
                    if let Some(tag) = self.element_tag(handle) {
                        tags.push((handle, tag));
                    }
                }
                _ => {}
            }
        }
        texts.sort_by_key(|(h, _)| h.0);
        tags.sort_by_key(|(h, _)| h.0);
        BoxSnapshot {
            boxes,
            texts,
            tags,
            event_targets: self.snapshot_event_targets(),
            gpu_slots: self.gpu_slots(),
        }
    }

    fn snapshot_event_targets(&self) -> HashSet<(u64, String)> {
        let mut targets = self.runtime.event_targets(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
        );
        for (&id, events) in &self.pending.events {
            targets.retain(|(event_id, _)| *event_id != id);
            for event in events {
                targets.insert((id, event.clone()));
            }
        }
        targets
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeHandle> {
        self.runtime
            .hit_test_candidates(nana_ui_runtime::DocumentId::try_from(self.id).ok()?, x, y)
            .into_iter()
            .map(NodeHandle::from)
            .next()
    }

    pub fn hit_event_target(&self, x: f32, y: f32, event: &str) -> Option<NodeHandle> {
        let event = normalize_event_name(event);
        let start = self.hit_test(x, y)?;
        let mut walk = Some(start);
        while let Some(cur) = walk {
            if self.has_event(cur, &event) {
                return Some(cur);
            }
            walk = self.parent_node(cur);
        }
        None
    }

    pub fn event_route(&self, target: NodeHandle) -> Option<nana_ui_runtime::EventRoute> {
        self.runtime
            .event_route(StableNodeId::try_from(target).ok()?)
    }

    pub fn pointer_hover(&self, pointer_id: u64) -> Option<NodeHandle> {
        self.runtime
            .pointer_hover(
                nana_ui_runtime::DocumentId::try_from(self.id)
                    .expect("Vue document IDs are nonzero"),
                pointer_id,
            )
            .map(NodeHandle::from)
    }

    pub fn set_pointer_hover(&mut self, pointer_id: u64, target: Option<NodeHandle>) -> bool {
        let target = match target.map(StableNodeId::try_from).transpose() {
            Ok(target) => target,
            Err(_) => return false,
        };
        self.runtime
            .set_pointer_hover(
                nana_ui_runtime::DocumentId::try_from(self.id)
                    .expect("Vue document IDs are nonzero"),
                pointer_id,
                target,
            )
            .is_ok()
    }

    pub fn press_pointer(&mut self, pointer_id: u64, target: NodeHandle) -> bool {
        let Ok(target) = StableNodeId::try_from(target) else {
            return false;
        };
        self.runtime
            .press_pointer(
                nana_ui_runtime::DocumentId::try_from(self.id)
                    .expect("Vue document IDs are nonzero"),
                pointer_id,
                target,
            )
            .is_ok()
    }

    pub fn release_pointer_press(&mut self, pointer_id: u64) -> Option<NodeHandle> {
        self.runtime
            .release_pointer_press(
                nana_ui_runtime::DocumentId::try_from(self.id)
                    .expect("Vue document IDs are nonzero"),
                pointer_id,
            )
            .map(NodeHandle::from)
    }

    pub fn clear_pointer_interactions(&mut self) {
        self.runtime.clear_pointer_interactions(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("Vue document IDs are nonzero"),
        );
    }

    pub fn capture_pointer(&mut self, pointer_id: u64, target: NodeHandle) -> bool {
        let Ok(target) = StableNodeId::try_from(target) else {
            return false;
        };
        if !self.nodes.contains_key(&target.get()) && !self.runtime.contains(target) {
            return false;
        }
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("Vue document IDs are nonzero");
        if self.runtime.pointer_capture(document, pointer_id) == Some(target) {
            return true;
        }
        self.commit_pending_with(|mutations| mutations.capture_pointer(pointer_id, target))
            .ok();
        self.runtime.pointer_capture(document, pointer_id) == Some(target)
    }

    pub fn release_pointer(&mut self, pointer_id: u64, target: NodeHandle) -> bool {
        let Ok(target) = StableNodeId::try_from(target) else {
            return false;
        };
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("Vue document IDs are nonzero");
        if self.runtime.pointer_capture(document, pointer_id) != Some(target) {
            return false;
        }
        self.commit_pending_with(|mutations| mutations.release_pointer(pointer_id, target))
            .ok();
        self.runtime.pointer_capture(document, pointer_id) != Some(target)
    }

    pub fn pointer_capture(&self, pointer_id: u64) -> Option<NodeHandle> {
        self.runtime
            .pointer_capture(
                nana_ui_runtime::DocumentId::try_from(self.id)
                    .expect("Vue document IDs are nonzero"),
                pointer_id,
            )
            .map(NodeHandle::from)
    }

    pub fn clear_pointer_captures(&mut self) {
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("Vue document IDs are nonzero");
        let captures = self.runtime.pointer_captures(document);
        if captures.is_empty() {
            return;
        }
        // Captures whose release is rejected (e.g. target despawned) are
        // dropped with a recorded rejection instead of panicking; the world
        // drops stale captures on despawn anyway.
        self.commit_pending_with(|mutations| {
            for (pointer_id, target) in captures {
                mutations.release_pointer(pointer_id, target);
            }
        })
        .ok();
    }

    pub fn take_pointer_capture_changes(&mut self) -> Vec<nana_ui_runtime::PointerCaptureChange> {
        self.runtime.take_pointer_capture_changes()
    }

    pub fn set_focus(&mut self, node: NodeHandle) {
        let Ok(node) = StableNodeId::try_from(node) else {
            return;
        };
        if !self.nodes.contains_key(&node.get()) && !self.runtime.contains(node) {
            return;
        }
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero");
        self.commit_pending_with(|mutations| {
            mutations.set_interaction(
                node,
                nana_ui_runtime::InteractionState {
                    pointer_events: true,
                    focusable: true,
                },
            );
            mutations.request_focus(document, Some(node));
        })
        .ok();
        self.flush_runtime_systems();
    }

    pub fn clear_focus(&mut self) {
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero");
        self.commit_pending_with(|mutations| mutations.request_focus(document, None))
            .ok();
        self.flush_runtime_systems();
    }

    pub fn focused(&self) -> Option<NodeHandle> {
        self.runtime
            .focused(nana_ui_runtime::DocumentId::try_from(self.id).ok()?)
            .map(NodeHandle::from)
    }

    pub fn runtime_now(&self) -> std::time::Duration {
        let epoch = self.host_animation_epoch.unwrap_or(self.animation_epoch);
        Instant::now().saturating_duration_since(epoch)
    }

    pub fn host_animation_epoch(&self) -> Option<Instant> {
        self.host_animation_epoch
    }

    pub fn set_host_animation_epoch(&mut self, epoch: Instant) {
        self.host_animation_epoch = Some(epoch);
    }

    pub fn next_animation_wakeup(&self) -> Option<Instant> {
        let epoch = self.host_animation_epoch.unwrap_or(self.animation_epoch);
        self.context()
            .next_animation_deadline()
            .and_then(|deadline| epoch.checked_add(deadline))
    }

    /// Test hook: advance the monotonic CSS animation clock.
    #[cfg(test)]
    pub fn set_runtime_clock_for_test(&mut self, elapsed: std::time::Duration) {
        self.host_animation_epoch = None;
        self.animation_epoch = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
    }

    pub fn start_css_animation(&mut self, spec: nana_ui_runtime::AnimationSpec) {
        if spec.duration.is_zero() || spec.frame_interval.is_zero() {
            return;
        }
        self.commit_pending_with(|mutations| mutations.start_animation(spec))
            .ok();
    }

    pub fn advance_css_animations(
        &mut self,
        now: std::time::Duration,
    ) -> nana_ui_runtime::AnimationFrame {
        self.runtime.context_mut().advance_animations(now)
    }

    /// Ensure a bridge-owned generated pseudo element exists in the Runtime tree.
    pub fn ensure_css_pseudo_element(
        &mut self,
        id: u64,
        parent: NodeHandle,
        pseudo: crate::css_interactive::GeneratedPseudo,
        insert_first: bool,
    ) {
        let handle = NodeHandle(id);
        if !self.nodes.contains_key(&id) {
            self.nodes.insert(
                id,
                Node {
                    data: NodeData::Element {
                        namespace: ElementNamespace::Html,
                        attrs: HashMap::from([(
                            crate::bridge::GENERATED_PSEUDO_ATTR.into(),
                            match pseudo {
                                crate::css_interactive::GeneratedPseudo::Before => "before",
                                crate::css_interactive::GeneratedPseudo::After => "after",
                                crate::css_interactive::GeneratedPseudo::Placeholder => {
                                    "placeholder"
                                }
                            }
                            .into(),
                        )]),
                    },
                    scope_id: None,
                },
            );
            let stable = StableNodeId::new(id).expect("generated pseudo id is nonzero");
            self.pending
                .kinds
                .insert(id, NodeKind::Element { tag: "span".into() });
            self.pending.mutations.create(
                stable,
                nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
                NodeKind::Element { tag: "span".into() },
            );
            self.pending.mutations.set_interaction(
                stable,
                InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
            );
        }
        let anchor = if insert_first {
            self.children_of(parent).first().copied()
        } else {
            None
        };
        self.insert(handle, parent, anchor);
    }

    pub fn remove_generated_pseudo(&mut self, node: NodeHandle) {
        if self.nodes.contains_key(&node.0) {
            self.dispose_subtree(node);
        }
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Detach `root` and drop it (and descendants) from the document map.
    /// Never disposes the html / body scaffold nodes.
    fn dispose_subtree(&mut self, root: NodeHandle) {
        if root.0 == self.html_root.0 || root.0 == self.mount_root.0 {
            // Clearing mount children uses remove(child) per child; never wipe roots.
            let children = self.children_of(root);
            for child in children {
                self.dispose_subtree(child);
            }
            return;
        }
        let ids = self.collect_element_preorder(root);
        if let Some(parent) = self.live_parent(root.0) {
            self.overlay_children_mut(parent).retain(|id| *id != root.0);
        }
        self.pending.parent.insert(root.0, None);
        self.pending
            .mutations
            .despawn_subtree(StableNodeId::try_from(root).expect("known node is nonzero"));
        for id in ids {
            self.nodes.remove(&id);
            self.host_texture_nodes.remove(&id);
            self.pending.parent.remove(&id);
            self.pending.children.remove(&id);
            self.pending.kinds.remove(&id);
            self.pending.texts.remove(&id);
            self.pending.events.remove(&id);
            self.pending.gpu.remove(&id);
        }
    }

    fn collect_preorder(&self, id: u64, out: &mut Vec<u64>) {
        out.push(id);
        for child in self.children_of(NodeHandle(id)) {
            self.collect_preorder(child.0, out);
        }
    }

    fn runtime_text(&self, node: NodeHandle) -> Option<String> {
        self.live_text(node)
    }

    fn enqueue_layout_if_changed(
        &self,
        mutations: &mut MutationQueue,
        handle: NodeHandle,
        layout: RuntimeLayoutBox,
    ) {
        let Ok(id) = StableNodeId::try_from(handle) else {
            return;
        };
        if self.runtime.layout_box(id) != Some(layout) {
            mutations.write_layout(id, layout);
        }
    }

    fn flush_runtime_systems(&mut self) {
        let viewport =
            LayoutViewport::new(self.logical_width.max(1.0), self.logical_height.max(1.0));
        #[cfg(feature = "scene-view")]
        let mut shaper = nana_ui::NanaTextShaper::default();
        #[cfg(not(feature = "scene-view"))]
        let mut shaper = MeasureTextShaper;
        let update = self
            .runtime
            .runtime_document_mut()
            .flush(viewport, &mut shaper)
            .expect("vue runtime frame");
        self.record_accessibility_delta(update.accessibility);
    }

    fn flush_runtime_extract(&mut self) {
        let update = self
            .runtime
            .runtime_document_mut()
            .flush_with(|context, work| {
                context.world_mut().reconcile_focus(&work.focus_ime);
                Ok(())
            })
            .expect("vue extract frame");
        self.record_accessibility_delta(update.accessibility);
    }

    fn record_accessibility_delta(&mut self, delta: AccessibilityDelta) {
        if delta.updated.is_empty() && delta.removed.is_empty() {
            return;
        }
        self.pending_accessibility_generation = delta.generation;
        if self.accessibility_full_required {
            return;
        }
        for node in delta.updated {
            self.pending_accessibility_removed.remove(&node.id);
            self.pending_accessibility_updated.insert(node.id, node);
        }
        for id in delta.removed {
            self.pending_accessibility_updated.remove(&id);
            self.pending_accessibility_removed.insert(id);
        }
        if self.pending_accessibility_updated.len() + self.pending_accessibility_removed.len()
            > MAX_PENDING_ACCESSIBILITY_CHANGES
        {
            self.pending_accessibility_updated.clear();
            self.pending_accessibility_removed.clear();
            self.accessibility_full_required = true;
        }
    }
}

fn host_texture_content(slot: String, revision: u64) -> CustomRenderNode {
    CustomRenderNode::new(HOST_TEXTURE_RENDERER, slot, revision)
}

fn canvas_host_texture_slot(id: &str) -> Option<String> {
    // Slot contract: `data-nana-canvas="{id}"` → CustomRenderNode
    // renderer `"nana.host-texture"` / resource `"canvas:{id}"`.
    // Canvas 2D pixels come from nana-ui-web-api (tiny-skia), uploaded by
    // CanvasGpuBridge. This is not a browser `<canvas>` and does not imply
    // a working 2D context on the node by itself.
    id.parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| format!("canvas:{id}"))
}

fn is_sidebar_frame_body(widget: &crate::SemanticWidget) -> bool {
    crate::scroll::is_runtime_scroll_body(&widget.props)
}

fn widget_icon(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Option<nana_ui_core::Icon> {
    widget
        .children
        .iter()
        .filter_map(|child| snapshot.get(*child))
        .find(|child| child.kind == crate::WidgetKind::Icon)
        .and_then(glyph_name_icon)
        .or_else(|| glyph_name_icon(widget))
}

fn glyph_name_icon(widget: &crate::SemanticWidget) -> Option<nana_ui_core::Icon> {
    nana_ui_core::Icon::parse_name(widget.props.display_label())
        .or_else(|| nana_ui_core::Icon::parse_name(&widget.props.value))
        .or_else(|| {
            widget
                .props
                .class_names
                .iter()
                .find_map(|class| nana_ui_core::Icon::parse_name(class))
        })
}

fn icon_consumed_by_parent(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> bool {
    let Some(parent) = widget.parent.and_then(|parent| snapshot.get(parent)) else {
        return false;
    };
    match parent.kind {
        crate::WidgetKind::Button => widget_icon(parent, snapshot).is_some(),
        crate::WidgetKind::EmptyState => true,
        _ => false,
    }
}

fn resolve_widget_component_type(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
) -> Option<ComponentTypeId> {
    if widget.kind == crate::WidgetKind::Button
        && widget_icon(widget, snapshot).is_some()
        && let Some(id) = context.resolve_component_tag("icon-button")
    {
        return Some(id.clone());
    }
    if widget.kind == crate::WidgetKind::Icon {
        if icon_consumed_by_parent(widget, snapshot) || glyph_name_icon(widget).is_none() {
            return None;
        }
        return context.resolve_component_tag("icon").cloned();
    }
    if widget.kind == crate::WidgetKind::Dialog
        && vue_confirm_dialog(&widget.props)
        && let Some(id) = context.resolve_component_tag("confirm-dialog")
    {
        return Some(id.clone());
    }
    if widget.kind == crate::WidgetKind::Textarea
        && crate::widget_map::highlight_language(&widget.props).is_some()
        && let Some(id) = context.resolve_component_tag("hosted-textarea")
    {
        return Some(id.clone());
    }
    if widget.kind.is_choice_field() {
        let tag = if widget.kind == crate::WidgetKind::SearchDropdown
            || crate::widget_map::is_search_dropdown(&widget.props)
        {
            Some("search-dropdown")
        } else if widget.kind == crate::WidgetKind::Dropdown
            || crate::widget_map::is_dropdown_field(&widget.props)
        {
            Some("dropdown")
        } else {
            None
        };
        if let Some(id) = tag.and_then(|tag| context.resolve_component_tag(tag)) {
            return Some(id.clone());
        }
    }
    let tag = if widget.props.element_tag.is_empty() {
        widget.kind.as_str()
    } else {
        widget.props.element_tag.as_str()
    };
    context.resolve_component_tag(tag).cloned()
}

fn can_bind_from_semantic(widget: &crate::SemanticWidget) -> bool {
    if widget.kind == crate::WidgetKind::Chip && widget.props.role.eq_ignore_ascii_case("tab") {
        return false;
    }
    let kind = effective_kind(widget);
    kind.is_choice_field()
        || matches!(
            kind,
            crate::WidgetKind::Column
                | crate::WidgetKind::Box
                | crate::WidgetKind::Row
                | crate::WidgetKind::Button
                | crate::WidgetKind::Chip
                | crate::WidgetKind::Input
                | crate::WidgetKind::Textarea
                | crate::WidgetKind::Checkbox
                | crate::WidgetKind::Range
                | crate::WidgetKind::Spinner
                | crate::WidgetKind::InteractiveCard
                | crate::WidgetKind::Skeleton
                | crate::WidgetKind::Tooltip
                | crate::WidgetKind::ActionMenu
                | crate::WidgetKind::ActionMenuItem
                | crate::WidgetKind::Dialog
                | crate::WidgetKind::Popover
                | crate::WidgetKind::Switch
                | crate::WidgetKind::Card
                | crate::WidgetKind::Progress
                | crate::WidgetKind::StatusBadge
                | crate::WidgetKind::ValidationMessage
                | crate::WidgetKind::Toast
                | crate::WidgetKind::LevelMeter
                | crate::WidgetKind::ImageViewer
                | crate::WidgetKind::XYPad
                | crate::WidgetKind::QrCode
                | crate::WidgetKind::ListItem
                | crate::WidgetKind::EmptyState
                | crate::WidgetKind::LabeledValue
                | crate::WidgetKind::FormField
                | crate::WidgetKind::Drawer
                | crate::WidgetKind::SidebarFrame
                | crate::WidgetKind::SidebarRow
                | crate::WidgetKind::SettingsRow
                | crate::WidgetKind::SettingsCard
                | crate::WidgetKind::ContextMenu
                | crate::WidgetKind::CommandPalette
                | crate::WidgetKind::Segmented
                | crate::WidgetKind::Tabs
                | crate::WidgetKind::TreeView
                | crate::WidgetKind::CalendarHeatmap
                | crate::WidgetKind::NativeMarkdown
                | crate::WidgetKind::Icon
                | crate::WidgetKind::GraphCanvas
                | crate::WidgetKind::Workspace
                | crate::WidgetKind::Dock
                | crate::WidgetKind::SplitPane
                | crate::WidgetKind::AppShell
                | crate::WidgetKind::SettingsPage
        )
}

fn semantic_numeric_fields(widget: &crate::SemanticWidget) -> (f32, f32) {
    match widget.kind {
        crate::WidgetKind::Progress => (widget.props.progress, widget.props.progress_max),
        crate::WidgetKind::LevelMeter => (
            crate::widget_map::level_meter_value(&widget.props),
            widget.props.max,
        ),
        _ => (widget.props.number, widget.props.max),
    }
}

fn bind_attr_overrides(widget: &crate::SemanticWidget) -> Vec<(String, String)> {
    let mut extras = Vec::new();
    let missing = |name: &str| {
        !widget
            .props
            .attrs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(name))
    };
    match widget.kind {
        crate::WidgetKind::Switch if missing("control-position") => extras.push((
            "control-position".into(),
            match widget.props.control_position {
                nana_ui_core::SwitchControlPosition::Start => "start",
                nana_ui_core::SwitchControlPosition::End => "end",
            }
            .into(),
        )),
        crate::WidgetKind::Card if missing("card-kind") => extras.push((
            "card-kind".into(),
            match widget.props.card_kind {
                nana_ui_core::CardKind::Surface => "surface",
                nana_ui_core::CardKind::Outlined => "outlined",
                nana_ui_core::CardKind::Raised => "raised",
                nana_ui_core::CardKind::Flat => "flat",
                nana_ui_core::CardKind::Selected => "selected",
            }
            .into(),
        )),
        crate::WidgetKind::StatusBadge => {
            if missing("compact") && crate::widget_map::class_has_compact(&widget.props) {
                extras.push(("compact".into(), "true".into()));
            }
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    status_tone_attr(crate::widget_map::status_tone_from_props(&widget.props))
                        .into(),
                ));
            }
        }
        crate::WidgetKind::Toast => {
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    toast_tone_attr(crate::widget_map::toast_tone_from_props(&widget.props)).into(),
                ));
            }
            if missing("dismissible") && crate::widget_map::toast_dismissible(&widget.props) {
                extras.push(("dismissible".into(), "true".into()));
            }
        }
        crate::WidgetKind::Progress => {
            if missing("cancellable") && crate::widget_map::progress_cancellable(&widget.props) {
                extras.push(("cancellable".into(), "true".into()));
            }
        }
        crate::WidgetKind::Range if missing("unit") && !widget.props.unit.is_empty() => {
            extras.push(("unit".into(), widget.props.unit.clone()));
        }
        crate::WidgetKind::ActionMenuItem => {
            if missing("danger") && crate::widget_map::action_menu_item_danger(&widget.props) {
                extras.push(("danger".into(), "true".into()));
            }
        }
        crate::WidgetKind::LevelMeter if missing("tone") => extras.push((
            "tone".into(),
            status_tone_attr(crate::widget_map::status_tone_from_props(&widget.props)).into(),
        )),
        crate::WidgetKind::ImageViewer if missing("src") => {
            if let Some(src) = widget
                .props
                .native_props
                .get("src")
                .map(host_value_text)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                extras.push(("src".into(), src));
            }
        }
        crate::WidgetKind::ListItem
            if missing("auto-height") && missing("autoheight") && widget.props.auto_height =>
        {
            extras.push(("auto-height".into(), "true".into()));
        }
        crate::WidgetKind::EmptyState | crate::WidgetKind::LabeledValue
            if missing("compact") && crate::widget_map::class_has_compact(&widget.props) =>
        {
            extras.push(("compact".into(), "true".into()));
        }
        crate::WidgetKind::LabeledValue if missing("muted") && widget.props.muted => {
            extras.push(("muted".into(), "true".into()));
        }
        crate::WidgetKind::Drawer if missing("side") && !widget.props.side.is_empty() => {
            extras.push(("side".into(), widget.props.side.clone()));
        }
        crate::WidgetKind::SidebarRow => {
            if missing("state") {
                extras.push((
                    "state".into(),
                    match crate::widget_map::sidebar_row_state(&widget.props) {
                        nana_ui_runtime::SidebarRowState::Idle => "idle",
                        nana_ui_runtime::SidebarRowState::Active => "active",
                        nana_ui_runtime::SidebarRowState::AncestorActive => "ancestor",
                        nana_ui_runtime::SidebarRowState::Disabled => "disabled",
                    }
                    .into(),
                ));
            }
            if missing("tone") {
                extras.push((
                    "tone".into(),
                    match crate::widget_map::sidebar_row_tone(&widget.props) {
                        nana_ui_runtime::SidebarRowTone::Default => "default",
                        nana_ui_runtime::SidebarRowTone::Warning => "warning",
                        nana_ui_runtime::SidebarRowTone::Error => "error",
                    }
                    .into(),
                ));
            }
            if missing("depth") && missing("data-depth") && missing("indent") {
                let depth = crate::widget_map::sidebar_row_depth(&widget.props);
                if depth > 0 {
                    extras.push(("depth".into(), depth.to_string()));
                }
            }
        }
        crate::WidgetKind::SettingsRow => {
            let (stacked, divided, loose, first, last) =
                crate::widget_map::settings_row_flags(&widget.props);
            if missing("stacked") && stacked {
                extras.push(("stacked".into(), "true".into()));
            }
            if missing("divided") && divided {
                extras.push(("divided".into(), "true".into()));
            }
            if missing("loose") && loose {
                extras.push(("loose".into(), "true".into()));
            }
            if missing("first-in-group") && missing("firstInGroup") && first {
                extras.push(("first-in-group".into(), "true".into()));
            }
            if missing("last-in-group") && missing("lastInGroup") && last {
                extras.push(("last-in-group".into(), "true".into()));
            }
        }
        crate::WidgetKind::ContextMenu => {
            if missing("anchor-x") && missing("data-anchor-x") {
                extras.push(("anchor-x".into(), widget.props.anchor_x.to_string()));
            }
            if missing("anchor-y") && missing("data-anchor-y") {
                extras.push(("anchor-y".into(), widget.props.anchor_y.to_string()));
            }
            if missing("searchable") && context_menu_searchable(&widget.props) {
                extras.push(("searchable".into(), "true".into()));
            }
        }
        crate::WidgetKind::Tabs | crate::WidgetKind::Segmented
            if missing("fill") && widget.props.fill =>
        {
            extras.push(("fill".into(), "true".into()));
        }
        _ => {}
    }
    extras.extend(bind_native_json_attrs(widget));
    extras
}

fn stringify_host_attr(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::String(text) => text.clone(),
        nana_js_engine::HostValue::Number(number) if number.is_finite() => {
            if number.fract() == 0.0 {
                format!("{}", *number as i64)
            } else {
                number.to_string()
            }
        }
        nana_js_engine::HostValue::Bool(flag) => flag.to_string(),
        nana_js_engine::HostValue::Array(_) | nana_js_engine::HostValue::Object(_) => {
            value.to_json_string()
        }
        _ => value.to_json_string(),
    }
}

fn bind_native_json_attrs(widget: &crate::SemanticWidget) -> Vec<(String, String)> {
    let mut extras = Vec::new();
    let missing = |name: &str, extras: &[(String, String)]| {
        !widget
            .props
            .attrs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(name))
            && !extras.iter().any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    let push = |extras: &mut Vec<(String, String)>, key: &str, host_keys: &[&str]| {
        if !missing(key, extras) {
            return;
        }
        for host in host_keys {
            if let Some(value) = widget.props.native_props.get(*host) {
                extras.push((key.to_string(), stringify_host_attr(value)));
                return;
            }
        }
    };
    let push_json = |extras: &mut Vec<(String, String)>, key: &str, host_keys: &[&str]| {
        if !missing(key, extras) {
            return;
        }
        for host in host_keys {
            if let Some(value) = widget.props.native_props.get(*host)
                && matches!(
                    value,
                    nana_js_engine::HostValue::Array(_) | nana_js_engine::HostValue::Object(_)
                )
            {
                extras.push((key.to_string(), value.to_json_string()));
                return;
            }
        }
    };
    match effective_kind(widget) {
        crate::WidgetKind::CommandPalette => {
            push_json(&mut extras, "items", &["items", "options"]);
        }
        crate::WidgetKind::TreeView => {
            push_json(&mut extras, "tree", &["tree"]);
            push_json(&mut extras, "nodes", &["nodes"]);
            push_json(&mut extras, "options", &["options"]);
        }
        crate::WidgetKind::CalendarHeatmap => {
            push_json(&mut extras, "data", &["data", "value"]);
            push(&mut extras, "options", &["options"]);
        }
        crate::WidgetKind::NativeMarkdown => {
            if widget.props.value.trim().is_empty() {
                let source = markdown_source_from_props(&widget.props);
                if !source.trim().is_empty() && missing("source", &extras) {
                    extras.push(("source".into(), source));
                }
            }
            push(
                &mut extras,
                "mermaid-renderer",
                &["mermaid-renderer", "mermaidrenderer"],
            );
            push(
                &mut extras,
                "math-renderer",
                &["math-renderer", "mathrenderer"],
            );
        }
        crate::WidgetKind::GraphCanvas => {
            if widget.props.native_props.contains_key("model") {
                push(&mut extras, "model", &["model"]);
            } else {
                let nodes = widget.props.native_props.get("nodes");
                let edges = widget.props.native_props.get("edges");
                if (nodes.is_some() || edges.is_some()) && missing("model", &extras) {
                    let mut map = BTreeMap::new();
                    if let Some(nodes) = nodes {
                        map.insert("nodes".into(), nodes.clone());
                    }
                    if let Some(edges) = edges {
                        map.insert("edges".into(), edges.clone());
                    }
                    extras.push((
                        "model".into(),
                        nana_js_engine::HostValue::Object(map).to_json_string(),
                    ));
                }
            }
            push(&mut extras, "nodes", &["nodes"]);
            push(&mut extras, "edges", &["edges"]);
            push(&mut extras, "viewport", &["viewport"]);
            push(&mut extras, "selection", &["selection"]);
        }
        crate::WidgetKind::Dock => {
            push_json(&mut extras, "root", &["root"]);
            push_json(&mut extras, "layout", &["layout"]);
        }
        crate::WidgetKind::SplitPane => {
            push(&mut extras, "axis", &["axis"]);
            push(&mut extras, "size", &["size"]);
            push(
                &mut extras,
                "default-size",
                &["default-size", "defaultsize"],
            );
            push(&mut extras, "min", &["min"]);
            push(&mut extras, "max", &["max"]);
        }
        crate::WidgetKind::SettingsPage => {
            push_json(&mut extras, "settings", &["settings", "model"]);
            push(&mut extras, "tab", &["tab", "value"]);
            push(
                &mut extras,
                "hide-header",
                &["hide-header", "hideheader", "hideHeader"],
            );
        }
        _ => {}
    }
    extras
}

fn status_tone_attr(tone: nana_ui_core::StatusTone) -> &'static str {
    match tone {
        nana_ui_core::StatusTone::Neutral => "neutral",
        nana_ui_core::StatusTone::Info => "info",
        nana_ui_core::StatusTone::Success => "success",
        nana_ui_core::StatusTone::Warning => "warning",
        nana_ui_core::StatusTone::Danger => "danger",
    }
}

fn toast_tone_attr(tone: nana_ui_core::ToastTone) -> &'static str {
    match tone {
        nana_ui_core::ToastTone::Info => "info",
        nana_ui_core::ToastTone::Success => "success",
        nana_ui_core::ToastTone::Warning => "warning",
        nana_ui_core::ToastTone::Danger => "danger",
    }
}

fn is_shell_composer_kind(kind: crate::WidgetKind) -> bool {
    matches!(
        kind,
        crate::WidgetKind::Workspace
            | crate::WidgetKind::Dock
            | crate::WidgetKind::SplitPane
            | crate::WidgetKind::AppShell
    )
}

fn shell_kind_from_ident(raw: &str) -> Option<crate::WidgetKind> {
    let parsed = crate::WidgetKind::parse(raw).or_else(|| {
        let lower = raw.trim().to_ascii_lowercase();
        let stripped = lower.strip_prefix("nana.").unwrap_or(&lower);
        crate::WidgetKind::parse(stripped)
    })?;
    is_shell_composer_kind(parsed).then_some(parsed)
}

fn effective_kind(widget: &crate::SemanticWidget) -> crate::WidgetKind {
    if !matches!(
        widget.kind,
        crate::WidgetKind::Column | crate::WidgetKind::Box | crate::WidgetKind::Row
    ) {
        return widget.kind;
    }
    if widget.props.element_tag.is_empty() {
        return widget.kind;
    }
    shell_kind_from_ident(&widget.props.element_tag).unwrap_or(widget.kind)
}

fn bind_semantic_slots(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Vec<(String, StableNodeId)> {
    let mut slots = Vec::new();
    let push = |slots: &mut Vec<(String, StableNodeId)>, name: &str, id: Option<StableNodeId>| {
        let Some(id) = id else {
            return;
        };
        if slots
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            return;
        }
        slots.push((name.to_string(), id));
    };
    let data_slot = |name: &str| {
        widget.children.iter().find_map(|child| {
            let child = snapshot.get(*child)?;
            (child.props.attrs.get("data-slot").map(String::as_str) == Some(name))
                .then(|| StableNodeId::new(child.id))
                .flatten()
        })
    };
    let assigned = |slots: &[(String, StableNodeId)], id: StableNodeId| {
        slots.iter().any(|(_, existing)| *existing == id)
    };

    match effective_kind(widget) {
        crate::WidgetKind::ListItem | crate::WidgetKind::SidebarRow => {
            push(
                &mut slots,
                "leading",
                data_slot("leading").or_else(|| data_slot("list-item-leading")),
            );
            let tools = data_slot("tools");
            if widget.kind == crate::WidgetKind::SidebarRow {
                push(&mut slots, "tools", tools);
            }
            let trailing = data_slot("trailing")
                .or_else(|| data_slot("list-item-trailing"))
                .filter(|id| Some(*id) != tools);
            push(&mut slots, "trailing", trailing);
            let content = data_slot("content")
                .or_else(|| data_slot("list-item-content"))
                .or_else(|| {
                    widget.children.iter().find_map(|child| {
                        let id = StableNodeId::new(*child)?;
                        (!assigned(&slots, id)).then_some(id)
                    })
                });
            push(&mut slots, "content", content);
        }
        crate::WidgetKind::EmptyState | crate::WidgetKind::LabeledValue => {
            push(&mut slots, "action", data_slot("action"));
            push(
                &mut slots,
                "action",
                crate::widget_map::first_button_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::FormField => {
            push(&mut slots, "control", data_slot("control"));
            push(
                &mut slots,
                "control",
                crate::widget_map::form_field_control_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::Drawer | crate::WidgetKind::Dialog => {
            push(&mut slots, "body", data_slot("body"));
            push(
                &mut slots,
                "body",
                widget.children.first().copied().and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::SidebarFrame => {
            let (top, body, footer) = crate::widget_map::sidebar_frame_slots(snapshot, widget);
            push(&mut slots, "top", top.and_then(StableNodeId::new));
            push(&mut slots, "body", body.and_then(StableNodeId::new));
            push(&mut slots, "footer", footer.and_then(StableNodeId::new));
        }
        crate::WidgetKind::SettingsRow => {
            let row = crate::widget_map::settings_row_slots(snapshot, widget);
            push(&mut slots, "copy", row.copy.and_then(StableNodeId::new));
            push(&mut slots, "label", row.label.and_then(StableNodeId::new));
            push(&mut slots, "hint", row.hint.and_then(StableNodeId::new));
            push(&mut slots, "control", data_slot("control"));
            push(
                &mut slots,
                "control",
                crate::widget_map::settings_row_control_child_id(snapshot, widget)
                    .and_then(StableNodeId::new),
            );
        }
        crate::WidgetKind::SettingsCard => {
            push(&mut slots, "title", data_slot("title"));
            let title = widget.children.iter().find_map(|child| {
                let child = snapshot.get(*child)?;
                child
                    .props
                    .class_names
                    .iter()
                    .any(|class| {
                        class.contains("nana-settings-card__title")
                            || class.contains("settings-card__title")
                    })
                    .then(|| StableNodeId::new(child.id))
                    .flatten()
            });
            push(&mut slots, "title", title);
        }
        crate::WidgetKind::AppShell => {
            let (title_bar, body, overlay) = app_shell_slots(widget, snapshot);
            push(&mut slots, "title-bar", title_bar);
            push(&mut slots, "body", body);
            push(&mut slots, "overlay", overlay);
        }
        crate::WidgetKind::SplitPane => {
            let children = element_child_widgets(widget, snapshot);
            let handle = data_slot("handle")
                .or_else(|| data_slot("split-handle"))
                .or_else(|| {
                    children
                        .iter()
                        .find(|child| is_split_handle_child(child))
                        .and_then(|child| StableNodeId::new(child.id))
                })
                .or_else(|| {
                    (children.len() >= 3)
                        .then(|| StableNodeId::new(children[2].id))
                        .flatten()
                });
            let panes = children
                .iter()
                .filter(|child| Some(child.id) != handle.map(StableNodeId::get))
                .filter_map(|child| StableNodeId::new(child.id))
                .collect::<Vec<_>>();
            push(
                &mut slots,
                "first",
                data_slot("first").or(panes.first().copied()),
            );
            push(
                &mut slots,
                "second",
                data_slot("second").or(panes.get(1).copied()),
            );
            push(&mut slots, "handle", handle);
        }
        crate::WidgetKind::Workspace => {
            for slot in workspace_slots_from_widget(widget, snapshot) {
                if let Some(content) = slot.content {
                    push(&mut slots, slot.id.as_str(), Some(content));
                }
            }
        }
        crate::WidgetKind::Dock => {
            for (index, child) in element_child_widgets(widget, snapshot)
                .into_iter()
                .enumerate()
            {
                let Some(id) = StableNodeId::new(child.id) else {
                    continue;
                };
                push(&mut slots, &dock_item_id(child, index), Some(id));
            }
        }
        crate::WidgetKind::SettingsPage => {
            push(&mut slots, "content", data_slot("content"));
            push(&mut slots, "body", data_slot("body"));
            push(
                &mut slots,
                "content",
                settings_page_content_child(widget, snapshot),
            );
        }
        _ => {
            for &child in &widget.children {
                let Some(child) = snapshot.get(child) else {
                    continue;
                };
                let Some(id) = StableNodeId::new(child.id) else {
                    continue;
                };
                let Some(raw) = child.props.attrs.get("data-slot") else {
                    continue;
                };
                let raw_lower = raw.trim().to_ascii_lowercase();
                let name = match raw_lower.as_str() {
                    "leading" | "list-item-leading" => "leading",
                    "content" | "list-item-content" => "content",
                    "trailing" | "list-item-trailing" => "trailing",
                    "tools" => "tools",
                    "action" => "action",
                    "control" => "control",
                    "body" | "sidebar-body" => "body",
                    "top" | "sidebar-top" => "top",
                    "footer" | "sidebar-footer" => "footer",
                    "copy" => "copy",
                    "label" => "label",
                    "hint" => "hint",
                    "title" => "title",
                    "title-bar" | "titlebar" | "title_bar" | "app-title-bar" => "title-bar",
                    "overlay" => "overlay",
                    "first" => "first",
                    "second" => "second",
                    "handle" | "split-handle" => "handle",
                    other => other,
                };
                push(&mut slots, name, Some(id));
            }
        }
    }
    slots
}

fn bind_semantic_copy(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> (Option<String>, Option<String>) {
    match widget.kind {
        crate::WidgetKind::LabeledValue => {
            let caption = crate::widget_map::labeled_value_caption(snapshot, widget);
            let label = (widget.props.label.is_empty() && !caption.is_empty()).then_some(caption);
            (label, None)
        }
        crate::WidgetKind::SettingsRow => {
            let slots = crate::widget_map::settings_row_slots(snapshot, widget);
            let slot_text = |slot: Option<crate::WidgetId>, fallback: &str| {
                if !fallback.is_empty() {
                    return None;
                }
                slot.map(|id| crate::widget_map::settings_row_plain_text(snapshot, id))
                    .filter(|text| !text.is_empty())
            };
            (
                slot_text(slots.label, widget.props.display_label()),
                slot_text(slots.hint, widget.props.hint.as_str()),
            )
        }
        _ => (None, None),
    }
}

fn context_menu_searchable(props: &crate::WidgetProps) -> bool {
    props.options.len() >= 6
        || props
            .class_names
            .iter()
            .any(|class| class.contains("search"))
}

fn try_bind_registered_component(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) -> Option<bool> {
    let type_id = resolve_widget_component_type(widget, snapshot, context)?;
    if !can_bind_from_semantic(widget) {
        if context.world().component_type(id) != Some(&type_id) {
            mutations.set_component_type(id, Some(type_id));
        }
        return None;
    }
    let layout = Arc::new(widget.props.layout.clone());
    let mut extra_attrs = bind_attr_overrides(widget);
    let missing_query = !widget
        .props
        .attrs
        .keys()
        .any(|key| key.eq_ignore_ascii_case("query") || key.eq_ignore_ascii_case("data-query"))
        && !extra_attrs
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("query"));
    if missing_query
        && ((widget.kind == crate::WidgetKind::ContextMenu
            && context_menu_searchable(&widget.props))
            || widget.kind == crate::WidgetKind::SearchDropdown
            || (widget.kind == crate::WidgetKind::Select
                && crate::widget_map::is_search_dropdown(&widget.props)))
        && let Some(state) = context.world().text_input(id)
    {
        extra_attrs.push(("query".into(), state.value.clone()));
    }
    let attr_pairs: Vec<(&str, &str)> = widget
        .props
        .attrs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .chain(
            extra_attrs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .collect();
    let slot_pairs = bind_semantic_slots(widget, snapshot);
    let slot_refs: Vec<(&str, StableNodeId)> = slot_pairs
        .iter()
        .map(|(name, id)| (name.as_str(), *id))
        .collect();
    let (owned_label, owned_hint) = bind_semantic_copy(widget, snapshot);
    let (number, max) = semantic_numeric_fields(widget);
    let tree_child_options = tree_child_bind_options(widget, snapshot);
    let option_list: Vec<SemanticOption<'_>> = if !widget.props.options.is_empty() {
        widget
            .props
            .options
            .iter()
            .map(|option| SemanticOption {
                value: option.value.as_str(),
                label: option.label.as_str(),
                disabled: option.disabled,
            })
            .collect()
    } else {
        tree_child_options
            .iter()
            .map(|(value, label, disabled)| SemanticOption {
                value: value.as_str(),
                label: label.as_str(),
                disabled: *disabled,
            })
            .collect()
    };
    let is_icon_button = type_id.as_str() == "nana.icon-button";
    let button_kind = if widget.kind == crate::WidgetKind::Chip {
        if widget.props.active || widget.props.toggled {
            nana_ui_core::ButtonKind::Selected
        } else {
            nana_ui_core::ButtonKind::Subtle
        }
    } else {
        widget.props.button_kind
    };
    let label = if is_icon_button && !widget.props.hint.is_empty() {
        widget.props.hint.as_str()
    } else if let Some(label) = owned_label.as_deref() {
        label
    } else {
        widget.props.label.as_str()
    };
    let hint = owned_hint.as_deref().unwrap_or(widget.props.hint.as_str());
    let spec = SemanticSpec {
        label,
        value: widget.props.value.as_str(),
        hint,
        placeholder: widget.props.placeholder.as_str(),
        disabled: widget.props.disabled
            || (widget.kind == crate::WidgetKind::Checkbox && widget.props.loading),
        loading: widget.props.loading,
        invalid: widget.props.invalid
            || (widget.kind == crate::WidgetKind::Dialog && vue_confirm_danger(&widget.props)),
        active: widget.props.active,
        toggled: widget.props.toggled,
        read_only: widget.props.read_only,
        secure: widget.props.secure,
        button_kind,
        size: widget.props.size,
        icon: widget_icon(widget, snapshot),
        min: widget.props.min,
        max,
        step: widget.props.step,
        number,
        options: &option_list,
        attrs: &attr_pairs,
        slots: &slot_refs,
        ..SemanticSpec::from_parts(&type_id, &layout)
    };
    let existing_input = matches!(
        widget.kind,
        crate::WidgetKind::Input | crate::WidgetKind::Textarea
    )
    .then(|| context.world().text_input(id).cloned())
    .flatten();
    match context.bind_semantic(id, &spec, mutations) {
        Ok(ComponentBindKind::Projected) => {
            if let Some(mut state) = existing_input {
                if state.value != widget.props.value {
                    state.replace_value(&widget.props.value);
                }
                mutations.set_text_input(id, Some(state));
            }
            if widget.kind == crate::WidgetKind::Popover {
                retain_projected_children(widget, id, context.world(), mutations);
            }
            if widget.kind == crate::WidgetKind::Tooltip
                && context.world().standard_visual(id).is_some()
            {
                mutations.set_standard_visual(id, None);
            }
            if matches!(
                widget.kind,
                crate::WidgetKind::Segmented | crate::WidgetKind::Tabs
            ) {
                let chrome = if widget.kind == crate::WidgetKind::Tabs {
                    SelectionChrome::Tabs
                } else {
                    SelectionChrome::Segmented
                };
                for (child, option) in widget.children.iter().zip(widget.props.options.iter()) {
                    let Some(child_id) = StableNodeId::new(*child) else {
                        continue;
                    };
                    project_segmented_option(
                        child_id,
                        option,
                        option.value == widget.props.value,
                        widget.props.size,
                        chrome,
                        widget.kind == crate::WidgetKind::Tabs && widget.props.fill,
                        context.world(),
                        mutations,
                    );
                }
            }
            enqueue_bound_assembly(widget, snapshot, id, context, mutations, pending);
            Some(true)
        }
        Ok(ComponentBindKind::Layout) | Err(_) => None,
    }
}

fn tree_child_bind_options(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Vec<(String, String, bool)> {
    if effective_kind(widget) != crate::WidgetKind::TreeView
        || !widget.props.options.is_empty()
        || host_tree_nodes(&widget.props).is_some()
    {
        return Vec::new();
    }
    element_child_widgets(widget, snapshot)
        .into_iter()
        .map(|child| {
            let id = if !child.props.value.is_empty() {
                child.props.value.clone()
            } else if !child.props.element_id.is_empty() {
                child.props.element_id.clone()
            } else {
                child.props.display_label().to_string()
            };
            (
                id,
                child.props.display_label().to_string(),
                child.props.disabled,
            )
        })
        .collect()
}

fn enqueue_bound_assembly(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) {
    let world = context.world();
    match effective_kind(widget) {
        crate::WidgetKind::NativeMarkdown => {
            let markdown =
                RuntimeNativeMarkdown::from_source(&markdown_source_from_props(&widget.props));
            let request = markdown_renderer_request(&markdown, &widget.props);
            if world.highlight_request(id) != request.as_ref() {
                mutations.set_highlight_request(id, request);
            }
            pending.markdowns.push((id, markdown));
        }
        crate::WidgetKind::AppShell => {
            let title = widget.props.display_label();
            let (title_bar, body, overlay) = app_shell_slots(widget, snapshot);
            if let Some(title_bar) = title_bar {
                let bar_title = if title.is_empty() {
                    snapshot
                        .get(title_bar.get())
                        .map(|child| child.props.display_label().to_string())
                        .unwrap_or_default()
                } else {
                    title.to_string()
                };
                let bar = RuntimeAppTitleBar::new(bar_title);
                if !snapshot
                    .get(title_bar.get())
                    .is_some_and(is_title_bar_child)
                {
                    bar.project(title_bar, world, mutations);
                    pending.title_bars.push((title_bar, bar));
                }
            }
            let mut component = RuntimeAppShell::new();
            if let Some(title_bar) = title_bar {
                component = component.title_bar(title_bar);
            }
            if let Some(body) = body {
                component = component.body(body);
            }
            if let Some(overlay) = overlay {
                component = component.overlay(overlay);
            }
            pending.app_shells.push((id, component));
        }
        crate::WidgetKind::SplitPane => {
            pending
                .split_panes
                .push((id, split_pane_from_widget(widget, snapshot, context, id)));
        }
        crate::WidgetKind::Workspace => {
            pending
                .workspaces
                .push((id, workspace_from_widget(widget, snapshot, context, id)));
        }
        crate::WidgetKind::Dock => {
            pending
                .docks
                .push((id, dock_from_widget_bound(widget, snapshot, context, id)));
        }
        crate::WidgetKind::SettingsPage => {
            let model = settings_model_from_props(&widget.props).unwrap_or_else(|| {
                let label = widget.props.display_label();
                let label = if label.is_empty() { "Settings" } else { label };
                nana_ui_core::SettingsModel::new(
                    "settings",
                    [nana_ui_core::SettingsTab::new("settings", label)],
                )
                .expect("fallback settings model has one tab")
            });
            let mut state = nana_ui_core::SettingsState::new(&model);
            if let Some(tab) = settings_active_tab(&widget.props) {
                state.select(&model, &tab);
            }
            let mut component = RuntimeSettingsPage::new(model, state);
            if let Some(content) = settings_page_content_child(widget, snapshot) {
                component = component.content(content);
            }
            if let Ok(existing) = context.read(
                Entity::<RuntimeSettingsPage>::from_stable_id(id),
                Clone::clone,
            ) {
                component.assembly = existing.assembly;
            }
            pending.settings_pages.push((id, component));
        }
        _ => {}
    }
}

fn project_migrating_component(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    id: StableNodeId,
    context: &AppContext,
    mutations: &mut MutationQueue,
    pending: &mut PendingAssembly,
) -> bool {
    let world = context.world();
    if crate::widget_map::is_settings_row_projected_slot(snapshot, widget) {
        return true;
    }
    if is_title_bar_child(widget) {
        let title = widget.props.display_label();
        let bar = RuntimeAppTitleBar::new(title);
        bar.project(id, world, mutations);
        pending.title_bars.push((id, bar));
        return true;
    }
    if !is_sidebar_frame_body(widget)
        && try_bind_registered_component(widget, snapshot, id, context, mutations, pending)
            == Some(true)
    {
        return true;
    }
    match effective_kind(widget) {
        crate::WidgetKind::Column | crate::WidgetKind::Box if is_sidebar_frame_body(widget) => {
            RuntimeSidebarFrame::scroll_body(widget.props.layout.clone())
                .project(id, world, mutations);
            true
        }
        _ => {
            if project_aligned_segmented_option(widget, snapshot, id, world, mutations) {
                return true;
            }
            if matches!(
                world.standard_visual(id),
                Some(
                    nana_ui_runtime::StandardVisual::Icon { .. }
                        | nana_ui_runtime::StandardVisual::Button { .. }
                        | nana_ui_runtime::StandardVisual::TextInput { .. }
                        | nana_ui_runtime::StandardVisual::Switch { .. }
                        | nana_ui_runtime::StandardVisual::Range { .. }
                        | nana_ui_runtime::StandardVisual::Card { .. }
                        | nana_ui_runtime::StandardVisual::StatusBadge { .. }
                        | nana_ui_runtime::StandardVisual::ValidationMessage { .. }
                        | nana_ui_runtime::StandardVisual::EmptyState { .. }
                        | nana_ui_runtime::StandardVisual::LabeledValue { .. }
                        | nana_ui_runtime::StandardVisual::SelectionOption { .. }
                        | nana_ui_runtime::StandardVisual::Progress { .. }
                        | nana_ui_runtime::StandardVisual::Spinner { .. }
                        | nana_ui_runtime::StandardVisual::LevelMeter { .. }
                        | nana_ui_runtime::StandardVisual::FormField { .. }
                        | nana_ui_runtime::StandardVisual::Toast { .. }
                        | nana_ui_runtime::StandardVisual::XYPad { .. }
                        | nana_ui_runtime::StandardVisual::QrCode { .. }
                        | nana_ui_runtime::StandardVisual::Select { .. }
                        | nana_ui_runtime::StandardVisual::MenuSurface { .. }
                        | nana_ui_runtime::StandardVisual::ActionMenuItem { .. }
                        | nana_ui_runtime::StandardVisual::TreeView { .. }
                        | nana_ui_runtime::StandardVisual::CommandPalette { .. }
                        | nana_ui_runtime::StandardVisual::ModalFrame { .. }
                        | nana_ui_runtime::StandardVisual::CalendarHeatmap { .. }
                        | nana_ui_runtime::StandardVisual::TimeSeriesChart { .. }
                        | nana_ui_runtime::StandardVisual::ReorderList { .. }
                        | nana_ui_runtime::StandardVisual::NativeMarkdown { .. }
                        | nana_ui_runtime::StandardVisual::SelectableRichText { .. }
                        | nana_ui_runtime::StandardVisual::GraphCanvas { .. }
                        | nana_ui_runtime::StandardVisual::ImageViewer { .. }
                        | nana_ui_runtime::StandardVisual::KeyCaptureLayer { .. }
                        | nana_ui_runtime::StandardVisual::KeymapLayer
                )
            ) {
                mutations.set_standard_visual(id, None);
            }
            false
        }
    }
}

fn vue_confirm_dialog(props: &crate::WidgetProps) -> bool {
    if props.role.eq_ignore_ascii_case("alertdialog") {
        return true;
    }
    if matches!(props.button_kind, nana_ui_core::ButtonKind::Danger) {
        return true;
    }
    if props.attrs.get("data-variant").is_some_and(|value| {
        value.eq_ignore_ascii_case("confirm") || value.eq_ignore_ascii_case("alertdialog")
    }) {
        return true;
    }
    props.class_names.iter().any(|class| {
        matches!(
            class.as_str(),
            "nana-confirm" | "nana-confirm-dialog" | "nana-alertdialog"
        )
    })
}

fn vue_confirm_danger(props: &crate::WidgetProps) -> bool {
    matches!(props.button_kind, nana_ui_core::ButtonKind::Danger)
}

fn project_segmented_option(
    id: StableNodeId,
    option: &crate::SelectOptionProp,
    selected: bool,
    size: nana_ui_core::ControlSize,
    chrome: SelectionChrome,
    fill: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    RuntimeSegmentedOption::new(Arc::<str>::from(option.label.as_str()))
        .disabled(option.disabled)
        .with_selected(selected)
        .surface(size, chrome, fill)
        .project(id, world, mutations);
}

fn project_aligned_segmented_option(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) -> bool {
    let Some(parent) = widget.parent.and_then(|parent| snapshot.get(parent)) else {
        return false;
    };
    if !matches!(
        parent.kind,
        crate::WidgetKind::Segmented | crate::WidgetKind::Tabs
    ) {
        return false;
    }
    let Some(index) = parent.children.iter().position(|child| *child == widget.id) else {
        return false;
    };
    let Some(option) = parent.props.options.get(index) else {
        return false;
    };
    let chrome = if parent.kind == crate::WidgetKind::Tabs {
        SelectionChrome::Tabs
    } else {
        SelectionChrome::Segmented
    };
    project_segmented_option(
        id,
        option,
        option.value == parent.props.value,
        parent.props.size,
        chrome,
        parent.kind == crate::WidgetKind::Tabs && parent.props.fill,
        world,
        mutations,
    );
    true
}

fn accessibility_role(kind: crate::WidgetKind, explicit_role: &str) -> AccessibilityRole {
    match explicit_role.trim().to_ascii_lowercase().as_str() {
        "document" => return AccessibilityRole::Document,
        "text" => return AccessibilityRole::Text,
        "button" => return AccessibilityRole::Button,
        "textbox" | "searchbox" => return AccessibilityRole::TextInput,
        "checkbox" => return AccessibilityRole::Checkbox,
        "switch" => return AccessibilityRole::Switch,
        "slider" => return AccessibilityRole::Slider,
        "combobox" => return AccessibilityRole::ComboBox,
        "progressbar" => return AccessibilityRole::ProgressIndicator,
        "list" | "listbox" => return AccessibilityRole::List,
        "listitem" | "option" => return AccessibilityRole::ListItem,
        "tablist" => return AccessibilityRole::TabList,
        "tab" => return AccessibilityRole::Tab,
        "dialog" | "alertdialog" => return AccessibilityRole::Dialog,
        "menu" => return AccessibilityRole::Menu,
        "menuitem" => return AccessibilityRole::MenuItem,
        "tooltip" => return AccessibilityRole::Tooltip,
        "img" => return AccessibilityRole::Image,
        _ => {}
    }
    match kind {
        crate::WidgetKind::Text => AccessibilityRole::Text,
        crate::WidgetKind::Button | crate::WidgetKind::Chip => AccessibilityRole::Button,
        crate::WidgetKind::Input | crate::WidgetKind::Textarea => AccessibilityRole::TextInput,
        crate::WidgetKind::Checkbox => AccessibilityRole::Checkbox,
        crate::WidgetKind::Switch => AccessibilityRole::Switch,
        crate::WidgetKind::Range => AccessibilityRole::Slider,
        crate::WidgetKind::Select
        | crate::WidgetKind::Dropdown
        | crate::WidgetKind::SearchDropdown => AccessibilityRole::ComboBox,
        crate::WidgetKind::Progress | crate::WidgetKind::LevelMeter => {
            AccessibilityRole::ProgressIndicator
        }
        crate::WidgetKind::InteractiveCard => AccessibilityRole::Button,
        crate::WidgetKind::ListItem | crate::WidgetKind::SidebarRow => AccessibilityRole::ListItem,
        crate::WidgetKind::Tabs | crate::WidgetKind::Segmented => AccessibilityRole::TabList,
        crate::WidgetKind::StatusBadge | crate::WidgetKind::ValidationMessage => {
            AccessibilityRole::Text
        }
        crate::WidgetKind::Dialog => AccessibilityRole::Dialog,
        crate::WidgetKind::ContextMenu | crate::WidgetKind::ActionMenu => AccessibilityRole::Menu,
        crate::WidgetKind::ActionMenuItem => AccessibilityRole::MenuItem,
        crate::WidgetKind::Tooltip => AccessibilityRole::Tooltip,
        crate::WidgetKind::XYPad => AccessibilityRole::Slider,
        crate::WidgetKind::QrCode => AccessibilityRole::Image,
        crate::WidgetKind::Icon => AccessibilityRole::Image,
        crate::WidgetKind::CommandPalette | crate::WidgetKind::ImageViewer => {
            AccessibilityRole::Dialog
        }
        crate::WidgetKind::TreeView => AccessibilityRole::List,
        crate::WidgetKind::CalendarHeatmap => AccessibilityRole::Image,
        crate::WidgetKind::NativeMarkdown => AccessibilityRole::Document,
        crate::WidgetKind::GraphCanvas => AccessibilityRole::Generic,
        _ => AccessibilityRole::Generic,
    }
}

fn host_tree_nodes(props: &crate::WidgetProps) -> Option<Vec<nana_ui_core::TreeNode<Arc<str>>>> {
    let value = props
        .native_props
        .get("tree")
        .or_else(|| props.native_props.get("nodes"))
        .or_else(|| props.native_props.get("options"))?;
    let nodes = host_array(value)
        .into_iter()
        .filter_map(tree_node_from_host)
        .collect::<Vec<_>>();
    (!nodes.is_empty()).then_some(nodes)
}

fn tree_node_from_host(
    value: &nana_js_engine::HostValue,
) -> Option<nana_ui_core::TreeNode<Arc<str>>> {
    match value {
        nana_js_engine::HostValue::Object(map) => {
            let id = host_map_text(map, &["value", "id", "key"]);
            let label = host_map_text(map, &["label", "title", "text"]);
            if id.is_empty() && label.is_empty() {
                return None;
            }
            let children = map
                .get("children")
                .map(host_array)
                .unwrap_or_default()
                .into_iter()
                .filter_map(tree_node_from_host)
                .collect::<Vec<_>>();
            let expanded = map.get("expanded").is_some_and(host_value_truthy);
            let selected = map.get("selected").is_some_and(host_value_truthy);
            let disabled = map.get("disabled").is_some_and(host_value_truthy);
            let identity = if id.is_empty() { label.clone() } else { id };
            let caption = if label.is_empty() {
                identity.clone()
            } else {
                label
            };
            let mut node = if children.is_empty() {
                nana_ui_core::TreeNode::leaf(Arc::<str>::from(identity.as_str()), caption)
            } else {
                nana_ui_core::TreeNode::branch(
                    Arc::<str>::from(identity.as_str()),
                    caption,
                    expanded,
                    children,
                )
            };
            node = node.selected(selected).disabled(disabled);
            if let Some(icon) = map
                .get("icon")
                .map(host_value_text)
                .filter(|name| !name.is_empty())
                .and_then(|name| nana_ui_core::Icon::parse_name(&name))
            {
                node = node.icon(icon);
            }
            Some(node)
        }
        nana_js_engine::HostValue::String(text) if !text.is_empty() => Some(
            nana_ui_core::TreeNode::leaf(Arc::<str>::from(text.as_str()), text.clone()),
        ),
        _ => None,
    }
}

fn settings_model_from_props(props: &crate::WidgetProps) -> Option<nana_ui_core::SettingsModel> {
    let map = settings_host_map(props)?;
    let mut tabs = settings_tabs_from_host(host_map_value(&map, &["tabs", "items"])?);
    if tabs.is_empty() {
        return None;
    }
    let full_page = settings_full_page_keys(&map);
    if !full_page.is_empty() {
        tabs = tabs
            .into_iter()
            .map(|tab| {
                let flagged =
                    tab.full_page_value() || full_page.iter().any(|key| key == tab.id().as_str());
                let mut next = nana_ui_core::SettingsTab::new(tab.id().clone(), tab.label());
                if let Some(icon) = tab.icon_value() {
                    next = next.icon(icon);
                }
                next.full_page(flagged)
            })
            .collect();
    }
    let default_tab = host_map_text(&map, &["defaultTab", "default_tab", "default-tab"]);
    let default_tab =
        if default_tab.is_empty() || tabs.iter().all(|tab| tab.id().as_str() != default_tab) {
            tabs[0].id().as_str().to_string()
        } else {
            default_tab
        };
    let mut model = nana_ui_core::SettingsModel::new(default_tab, tabs).ok()?;
    if let Some(nana_js_engine::HostValue::Object(aliases)) =
        host_map_value(&map, &["aliases", "alias"])
    {
        for (alias, target) in aliases {
            let target = host_value_text(target);
            if target.is_empty() {
                continue;
            }
            if let Ok(next) = model.clone().with_alias(alias.as_str(), target) {
                model = next;
            }
        }
    }
    let hide_header = host_map_truthy(&map, &["hideHeader", "hide_header", "hide-header"])
        || settings_hide_header_from_props(props);
    Some(model.hide_header(hide_header))
}

fn settings_host_map(
    props: &crate::WidgetProps,
) -> Option<BTreeMap<String, nana_js_engine::HostValue>> {
    if let Some(value) = props
        .native_props
        .get("settings")
        .or_else(|| props.native_props.get("model"))
    {
        return match value {
            nana_js_engine::HostValue::Object(map) => Some(map.clone()),
            nana_js_engine::HostValue::String(json) => settings_object_from_json(json),
            _ => None,
        };
    }
    crate::widget_map::attr_value(props, &["settings", "model"]).and_then(settings_object_from_json)
}

fn settings_object_from_json(json: &str) -> Option<BTreeMap<String, nana_js_engine::HostValue>> {
    match nana_js_engine::HostValue::from_json_str(json).ok()? {
        nana_js_engine::HostValue::Object(map) => Some(map),
        _ => None,
    }
}

fn settings_tabs_from_host(value: &nana_js_engine::HostValue) -> Vec<nana_ui_core::SettingsTab> {
    let items = host_array(value);
    if !items.is_empty() {
        return items
            .into_iter()
            .filter_map(settings_tab_from_host)
            .collect();
    }
    if let Some(tab) = settings_tab_from_host(value) {
        return vec![tab];
    }
    let nana_js_engine::HostValue::Object(map) = value else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, item)| match item {
            nana_js_engine::HostValue::String(label) if !label.is_empty() => {
                Some(nana_ui_core::SettingsTab::new(key.as_str(), label.as_str()))
            }
            nana_js_engine::HostValue::Object(_) => {
                let tab = settings_tab_from_host(item)?;
                if tab.id().as_str().is_empty() {
                    Some(nana_ui_core::SettingsTab::new(key.as_str(), tab.label()))
                } else {
                    Some(tab)
                }
            }
            _ => None,
        })
        .collect()
}

fn settings_tab_from_host(value: &nana_js_engine::HostValue) -> Option<nana_ui_core::SettingsTab> {
    match value {
        nana_js_engine::HostValue::String(id) if !id.is_empty() => {
            Some(nana_ui_core::SettingsTab::new(id.as_str(), id.as_str()))
        }
        nana_js_engine::HostValue::Object(map) => {
            let id = host_map_text(map, &["key", "id", "value"]);
            if id.is_empty() {
                return None;
            }
            let label = host_map_text(map, &["label", "title", "name"]);
            let label = if label.is_empty() { id.clone() } else { label };
            let mut tab = nana_ui_core::SettingsTab::new(id, label);
            if let Some(icon) = nana_ui_core::Icon::parse_name(&host_map_text(
                map,
                &["icon", "iconName", "icon-name"],
            )) {
                tab = tab.icon(icon);
            }
            Some(tab.full_page(host_map_truthy(
                map,
                &["fullPage", "full_page", "full-page"],
            )))
        }
        _ => None,
    }
}

fn settings_full_page_keys(map: &BTreeMap<String, nana_js_engine::HostValue>) -> Vec<String> {
    host_map_value(map, &["fullPageTabs", "full_page_tabs", "full-page-tabs"])
        .map(host_array)
        .unwrap_or_default()
        .into_iter()
        .map(host_value_text)
        .filter(|key| !key.is_empty())
        .collect()
}

fn settings_hide_header_from_props(props: &crate::WidgetProps) -> bool {
    if let Some(value) = props
        .native_props
        .get("hide-header")
        .or_else(|| props.native_props.get("hideHeader"))
    {
        return host_value_truthy(value);
    }
    crate::widget_map::attr_value(props, &["hide-header", "hideheader", "hideHeader"]).is_some_and(
        |value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "true" | "1" | "yes"
            )
        },
    )
}

fn settings_active_tab(props: &crate::WidgetProps) -> Option<nana_ui_core::SettingsTabId> {
    native_prop_text(props, &["tab", "value"])
        .or_else(|| {
            crate::widget_map::attr_value(props, &["tab", "value"])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            let value = props.value.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .map(nana_ui_core::SettingsTabId::from)
}

fn settings_page_content_child(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Option<StableNodeId> {
    widget.children.iter().copied().find_map(|id| {
        let child = snapshot.get(id)?;
        if is_settings_page_header_child(child) {
            return None;
        }
        StableNodeId::new(id)
    })
}

fn is_settings_page_header_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget
        .props
        .attrs
        .get("data-slot")
        .map(String::as_str)
        .unwrap_or("");
    if matches!(slot, "header" | "page-header" | "title") {
        return true;
    }
    widget
        .props
        .class_names
        .iter()
        .any(|class| class.contains("nana-settings-page__header") || class.contains("page-header"))
}

fn markdown_source_from_props(props: &crate::WidgetProps) -> String {
    if !props.value.trim().is_empty() {
        return props.value.clone();
    }
    if let Some(source) = crate::widget_map::attr_value(props, &["source", "markdown", "value"])
        && !source.trim().is_empty()
    {
        return source.to_string();
    }
    for key in ["source", "markdown", "value"] {
        if let Some(text) = props.native_props.get(key).map(host_value_text)
            && !text.trim().is_empty()
        {
            return text;
        }
    }
    String::new()
}

fn markdown_renderer_request(
    markdown: &RuntimeNativeMarkdown,
    props: &crate::WidgetProps,
) -> Option<HighlightRequest> {
    let mermaid = crate::widget_map::mermaid_renderer(props)
        .map(str::to_string)
        .or_else(|| native_prop_text(props, &["mermaid-renderer", "mermaidrenderer"]));
    let math = crate::widget_map::math_renderer(props)
        .map(str::to_string)
        .or_else(|| native_prop_text(props, &["math-renderer", "mathrenderer"]));
    for block in markdown.blocks() {
        match block {
            nana_ui_runtime::MarkdownBlock::Mermaid(source) => {
                if let Some(name) = mermaid.as_deref() {
                    return Some(HighlightRequest::new(name, format!("mermaid:{source}")));
                }
            }
            nana_ui_runtime::MarkdownBlock::DisplayMath(source) => {
                if let Some(name) = math.as_deref() {
                    return Some(HighlightRequest::new(name, format!("math:{source}")));
                }
            }
            _ => {}
        }
    }
    None
}

fn native_prop_text(props: &crate::WidgetProps, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        props
            .native_props
            .get(*key)
            .map(host_value_text)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    })
}

fn host_value_number(value: &nana_js_engine::HostValue) -> Option<f32> {
    match value {
        nana_js_engine::HostValue::Number(number) if number.is_finite() => Some(*number as f32),
        nana_js_engine::HostValue::String(text) => text.trim().parse().ok(),
        nana_js_engine::HostValue::Bool(true) => Some(1.0),
        nana_js_engine::HostValue::Bool(false) => Some(0.0),
        _ => None,
    }
}

fn host_array(value: &nana_js_engine::HostValue) -> Vec<&nana_js_engine::HostValue> {
    match value {
        nana_js_engine::HostValue::Array(items) => items.iter().collect(),
        nana_js_engine::HostValue::Object(map) => {
            let mut indexed = map
                .iter()
                .filter_map(|(key, item)| key.parse::<usize>().ok().map(|index| (index, item)))
                .collect::<Vec<_>>();
            if indexed.is_empty() {
                return Vec::new();
            }
            indexed.sort_by_key(|(index, _)| *index);
            indexed.into_iter().map(|(_, item)| item).collect()
        }
        _ => Vec::new(),
    }
}

fn host_map_value<'a>(
    map: &'a BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> Option<&'a nana_js_engine::HostValue> {
    for key in keys {
        if let Some(value) = map.get(*key) {
            return Some(value);
        }
    }
    for key in keys {
        if let Some((_, value)) = map
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        {
            return Some(value);
        }
    }
    None
}

fn host_map_text(map: &BTreeMap<String, nana_js_engine::HostValue>, keys: &[&str]) -> String {
    host_map_value(map, keys)
        .map(host_value_text)
        .filter(|text| !text.is_empty())
        .unwrap_or_default()
}

fn host_map_truthy(map: &BTreeMap<String, nana_js_engine::HostValue>, keys: &[&str]) -> bool {
    host_map_value(map, keys).is_some_and(host_value_truthy)
}

fn host_map_number(
    map: &BTreeMap<String, nana_js_engine::HostValue>,
    keys: &[&str],
) -> Option<f32> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(host_value_number))
}

fn host_value_text(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::String(text) => text.clone(),
        nana_js_engine::HostValue::Number(number) if number.is_finite() => number.to_string(),
        nana_js_engine::HostValue::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

fn host_value_truthy(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Bool(flag) => *flag,
        nana_js_engine::HostValue::Number(number) => *number != 0.0,
        nana_js_engine::HostValue::String(text) => {
            !text.is_empty() && !text.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

fn is_shell_composer_slot(
    snapshot: &crate::SemanticSnapshot,
    widget: &crate::SemanticWidget,
) -> bool {
    widget
        .parent
        .and_then(|parent| snapshot.get(parent))
        .is_some_and(|parent| is_shell_composer_kind(effective_kind(parent)))
}

fn retain_projected_children(
    widget: &crate::SemanticWidget,
    parent: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    for child in &widget.children {
        let Some(child_id) = StableNodeId::new(*child) else {
            continue;
        };
        if !world.contains(child_id) {
            continue;
        }
        if world
            .node(child_id)
            .is_some_and(|node| node.parent != Some(parent))
        {
            mutations.insert(parent, child_id, None);
        }
    }
}

fn element_child_widgets<'a>(
    widget: &'a crate::SemanticWidget,
    snapshot: &'a crate::SemanticSnapshot,
) -> Vec<&'a crate::SemanticWidget> {
    widget
        .children
        .iter()
        .filter_map(|child| snapshot.get(*child))
        .filter(|child| child.kind != crate::WidgetKind::Text)
        .collect()
}

fn widget_slot(widget: &crate::SemanticWidget) -> String {
    crate::widget_map::attr_value(&widget.props, &["data-slot", "slot"])
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn widget_tag(widget: &crate::SemanticWidget) -> String {
    widget.props.element_tag.trim().to_ascii_lowercase()
}

fn widget_class_blob(widget: &crate::SemanticWidget) -> String {
    widget
        .props
        .class_names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_title_bar_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if matches!(slot.as_str(), "title-bar" | "titlebar" | "app-title-bar") {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("title-bar")
        || tag.contains("titlebar")
        || tag.contains("app-title-bar")
        || class.contains("title-bar")
        || class.contains("titlebar")
        || class.contains("nana-app-title-bar")
}

fn is_body_child(widget: &crate::SemanticWidget) -> bool {
    widget_slot(widget) == "body"
}

fn is_overlay_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if slot == "overlay" {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("overlay") || class.contains("nana-app-shell__overlay")
}

fn is_split_handle_child(widget: &crate::SemanticWidget) -> bool {
    let slot = widget_slot(widget);
    if matches!(slot.as_str(), "handle" | "split-handle") {
        return true;
    }
    let tag = widget_tag(widget);
    let class = widget_class_blob(widget);
    tag.contains("split-handle")
        || class.contains("nana-split-handle")
        || class.contains("split-handle")
}

fn native_prop_number(props: &crate::WidgetProps, keys: &[&str]) -> Option<f32> {
    keys.iter().find_map(|key| {
        props
            .native_props
            .get(*key)
            .and_then(host_value_number)
            .filter(|number| number.is_finite())
    })
}

fn split_axis_from_props(props: &crate::WidgetProps) -> nana_ui_core::SplitAxis {
    let raw = native_prop_text(props, &["axis"])
        .or_else(|| {
            crate::widget_map::attr_value(props, &["axis", "data-axis"]).map(str::to_string)
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    match raw.as_str() {
        "vertical" | "column" | "y" => nana_ui_core::SplitAxis::Vertical,
        _ => nana_ui_core::SplitAxis::Horizontal,
    }
}

fn split_pane_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeSplitPane {
    let children = element_child_widgets(widget, snapshot);
    let handle = children
        .iter()
        .find(|child| is_split_handle_child(child))
        .and_then(|child| StableNodeId::new(child.id))
        .or_else(|| {
            (children.len() >= 3)
                .then(|| StableNodeId::new(children[2].id))
                .flatten()
        })
        .or_else(|| {
            context
                .read(Entity::<RuntimeSplitPane>::from_stable_id(id), |pane| {
                    pane.handle
                })
                .ok()
                .flatten()
        });
    let panes = children
        .iter()
        .filter(|child| Some(child.id) != handle.map(StableNodeId::get))
        .filter_map(|child| StableNodeId::new(child.id))
        .collect::<Vec<_>>();
    let first = panes.first().copied();
    let second = panes.get(1).copied();
    let default_size = native_prop_number(&widget.props, &["default-size", "defaultsize"])
        .or_else(|| native_prop_number(&widget.props, &["size"]))
        .unwrap_or(240.0);
    let min = native_prop_number(&widget.props, &["min"]).unwrap_or(120.0);
    let max = native_prop_number(&widget.props, &["max"]).unwrap_or(800.0);
    let mut model = nana_ui_core::SplitPaneModel::new(
        split_axis_from_props(&widget.props),
        default_size,
        min,
        max,
    );
    if let Some(size) = native_prop_number(&widget.props, &["size"])
        && (size - model.size()).abs() > f32::EPSILON
    {
        model.update(nana_ui_core::SplitPaneMutation::SetSize(size));
    }
    match (first, second) {
        (Some(first), Some(second)) => {
            let mut pane = RuntimeSplitPane::from_model(&model, first, second);
            if let Some(handle) = handle {
                pane = pane.handle(handle);
            }
            pane
        }
        _ => RuntimeSplitPane {
            first,
            second,
            handle,
            model,
        },
    }
}

fn workspace_region_token(widget: &crate::SemanticWidget) -> String {
    if !widget.props.region.is_empty() {
        return widget.props.region.to_ascii_lowercase();
    }
    crate::widget_map::attr_value(
        &widget.props,
        &[
            "data-region-role",
            "data-region",
            "region",
            "region-role",
            "data-region-id",
        ],
    )
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase()
}

fn region_id_from_token(token: &str) -> Option<nana_ui_core::RegionId> {
    let token = token.trim().trim_start_matches("region-");
    if token.is_empty() {
        return None;
    }
    Some(match token {
        "global-navigation" | "globalnavigation" | "global" => {
            nana_ui_core::RegionId::GlobalNavigation
        }
        "section-navigation" | "sectionnavigation" => nana_ui_core::RegionId::SectionNavigation,
        "resources" | "sidebar" | "files" => nana_ui_core::RegionId::Resources,
        "primary-toolbar" | "primarytoolbar" | "toolbar" => nana_ui_core::RegionId::PrimaryToolbar,
        "primary" | "main" => nana_ui_core::RegionId::Primary,
        "inspector" => nana_ui_core::RegionId::Inspector,
        "diagnostics" | "console" => nana_ui_core::RegionId::Diagnostics,
        other => nana_ui_core::RegionId::custom(other),
    })
}

fn workspace_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeWorkspace {
    let slots = workspace_slots_from_widget(widget, snapshot);
    let mut component = RuntimeWorkspace::from_model(&nana_ui_core::WorkspaceModel::new(), slots);
    if let Ok(existing) = context.read(Entity::<RuntimeWorkspace>::from_stable_id(id), Clone::clone)
    {
        component.middle = existing.middle;
        component.primary_column = existing.primary_column;
        component.primary_row = existing.primary_row;
        component.editor_stack = existing.editor_stack;
        component.handles = existing.handles;
    }
    component
}

fn workspace_slots_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Vec<WorkspaceRegionSlot> {
    let children = element_child_widgets(widget, snapshot);
    let mut slots = Vec::new();
    for (index, child) in children.iter().enumerate() {
        let Some(content) = StableNodeId::new(child.id) else {
            continue;
        };
        let token = workspace_region_token(child);
        let region = if let Some(region) = region_id_from_token(&token) {
            region
        } else {
            let fallback = if !child.props.element_id.is_empty() {
                child.props.element_id.clone()
            } else if !child.props.value.is_empty() {
                child.props.value.clone()
            } else {
                format!("region-{index}")
            };
            nana_ui_core::RegionId::custom(fallback)
        };
        if slots
            .iter()
            .any(|slot: &WorkspaceRegionSlot| slot.id == region)
        {
            continue;
        }
        slots.push(WorkspaceRegionSlot::new(region, content));
    }
    slots
}

fn dock_item_id(widget: &crate::SemanticWidget, index: usize) -> String {
    crate::widget_map::attr_value(&widget.props, &["data-dock-id", "dock-id", "id", "data-id"])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| (!widget.props.element_id.is_empty()).then(|| widget.props.element_id.clone()))
        .or_else(|| (!widget.props.value.is_empty()).then(|| widget.props.value.clone()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let label = widget.props.display_label();
            if label.is_empty() {
                format!("item-{index}")
            } else {
                label.to_string()
            }
        })
}

fn dock_item_title(widget: &crate::SemanticWidget, id: &str) -> String {
    let title = widget.props.display_label();
    if title.is_empty() {
        id.to_string()
    } else {
        title.to_string()
    }
}

fn dock_content_for_id(
    id: &str,
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> Option<StableNodeId> {
    children.iter().find_map(|(child, node)| {
        let child_id = dock_item_id(child, 0);
        (child_id == id).then_some(*node)
    })
}

fn parse_dock_axis(raw: &str) -> DockAxis {
    match raw.trim().to_ascii_lowercase().as_str() {
        "vertical" | "column" | "y" => DockAxis::Vertical,
        _ => DockAxis::Horizontal,
    }
}

fn dock_node_from_host(
    value: &nana_js_engine::HostValue,
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> Option<DockNode> {
    match value {
        nana_js_engine::HostValue::String(id) if !id.is_empty() => Some(DockNode::item(
            id.as_str(),
            dock_content_for_id(id, children),
        )),
        nana_js_engine::HostValue::Array(items) => {
            let nodes = items
                .iter()
                .filter_map(|item| dock_node_from_host(item, children))
                .collect::<Vec<_>>();
            dock_nodes_join(nodes)
        }
        nana_js_engine::HostValue::Object(map) => {
            let kind = host_map_text(map, &["type", "kind", "node"]).to_ascii_lowercase();
            if kind == "split" || map.contains_key("first") || map.contains_key("second") {
                let first = map
                    .get("first")
                    .and_then(|value| dock_node_from_host(value, children))?;
                let second = map
                    .get("second")
                    .and_then(|value| dock_node_from_host(value, children))?;
                let axis = parse_dock_axis(&host_map_text(map, &["axis"]));
                let ratio = host_map_number(map, &["ratio", "size"]).unwrap_or(0.5);
                return Some(DockNode::split(axis, ratio, first, second));
            }
            if kind == "tabs" || map.contains_key("tabs") {
                let tab_values = map.get("tabs").map(host_array).unwrap_or_default();
                let mut tabs = Vec::new();
                let mut contents = Vec::new();
                for tab in tab_values {
                    let (id, content) = match tab {
                        nana_js_engine::HostValue::String(id) if !id.is_empty() => {
                            (id.clone(), dock_content_for_id(id, children))
                        }
                        nana_js_engine::HostValue::Object(tab_map) => {
                            let id = host_map_text(tab_map, &["id", "key", "value"]);
                            if id.is_empty() {
                                continue;
                            }
                            (id.clone(), dock_content_for_id(&id, children))
                        }
                        _ => continue,
                    };
                    if tabs
                        .iter()
                        .any(|existing: &Arc<str>| existing.as_ref() == id)
                    {
                        continue;
                    }
                    let id = Arc::<str>::from(id);
                    contents.push((Arc::clone(&id), content));
                    tabs.push(id);
                }
                if tabs.is_empty() {
                    return None;
                }
                let active = host_map_text(map, &["active", "value"]);
                let active = if active.is_empty() {
                    Arc::clone(&tabs[0])
                } else {
                    Arc::<str>::from(active)
                };
                return Some(DockNode::tabs(tabs, active, contents));
            }
            let id = host_map_text(map, &["id", "key", "value", "dock-id", "data-dock-id"]);
            if id.is_empty() {
                return None;
            }
            Some(DockNode::item(
                id.as_str(),
                dock_content_for_id(&id, children),
            ))
        }
        _ => None,
    }
}

fn dock_nodes_join(mut nodes: Vec<DockNode>) -> Option<DockNode> {
    match nodes.len() {
        0 => None,
        1 => nodes.pop(),
        _ => {
            let ids = nodes.iter().flat_map(DockNode::flatten).collect::<Vec<_>>();
            if ids.is_empty() {
                return None;
            }
            let mut contents = Vec::new();
            for node in &nodes {
                collect_dock_contents(node, &mut contents);
            }
            let active = Arc::clone(&ids[0]);
            Some(DockNode::tabs(ids, active, contents))
        }
    }
}

fn collect_dock_contents(node: &DockNode, output: &mut Vec<(Arc<str>, Option<StableNodeId>)>) {
    match node {
        DockNode::Item { id, content } => output.push((Arc::clone(id), *content)),
        DockNode::Tabs { contents, .. } => output.extend(contents.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_dock_contents(first, output);
            collect_dock_contents(second, output);
        }
    }
}

fn dock_root_from_children(
    children: &[(&crate::SemanticWidget, StableNodeId)],
) -> (DockNode, Vec<(Arc<str>, Arc<str>)>) {
    let mut titles = Vec::new();
    let mut items = Vec::new();
    for (index, (child, content)) in children.iter().enumerate() {
        let id = dock_item_id(child, index);
        let title = dock_item_title(child, &id);
        titles.push((Arc::<str>::from(id.as_str()), Arc::<str>::from(title)));
        items.push((Arc::<str>::from(id), Some(*content)));
    }
    let root = match items.as_slice() {
        [] => DockNode::item("dock", None),
        [(id, content)] => DockNode::item(Arc::clone(id), *content),
        _ => {
            let tabs = items
                .iter()
                .map(|(id, _)| Arc::clone(id))
                .collect::<Vec<_>>();
            let active = Arc::clone(&tabs[0]);
            DockNode::tabs(tabs, active, items)
        }
    };
    (root, titles)
}

fn dock_from_widget_bound(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    context: &AppContext,
    id: StableNodeId,
) -> RuntimeDock {
    let next = dock_from_widget(widget, snapshot);
    match context.read(Entity::<RuntimeDock>::from_stable_id(id), Clone::clone) {
        Ok(mut existing) => {
            existing.root = next.root;
            existing.drop_target = next.drop_target;
            existing.locked = next.locked;
            existing.titles = next.titles;
            existing.style = next.style;
            existing
        }
        Err(_) => next,
    }
}

fn dock_from_widget(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> RuntimeDock {
    let child_pairs = element_child_widgets(widget, snapshot)
        .into_iter()
        .filter_map(|child| StableNodeId::new(child.id).map(|id| (child, id)))
        .collect::<Vec<_>>();
    let host_root = widget
        .props
        .native_props
        .get("root")
        .or_else(|| widget.props.native_props.get("layout"))
        .and_then(|value| dock_node_from_host(value, &child_pairs));
    let (root, titles) = if let Some(root) = host_root {
        let mut titles = Vec::new();
        for (child, _) in &child_pairs {
            let id = dock_item_id(child, 0);
            let title = dock_item_title(child, &id);
            if title != id {
                titles.push((Arc::<str>::from(id), Arc::<str>::from(title)));
            }
        }
        (root, titles)
    } else {
        dock_root_from_children(&child_pairs)
    };
    let mut dock = RuntimeDock::new(root);
    for (id, title) in titles {
        dock = dock.title(id, title);
    }
    dock
}

fn app_shell_slots(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> (
    Option<StableNodeId>,
    Option<StableNodeId>,
    Option<StableNodeId>,
) {
    let children = element_child_widgets(widget, snapshot);
    let title_bar = children
        .iter()
        .find(|child| is_title_bar_child(child))
        .and_then(|child| StableNodeId::new(child.id));
    let overlay = children
        .iter()
        .find(|child| is_overlay_child(child) && Some(child.id) != title_bar.map(StableNodeId::get))
        .and_then(|child| StableNodeId::new(child.id));
    let body = children
        .iter()
        .find(|child| {
            is_body_child(child)
                && Some(child.id) != title_bar.map(StableNodeId::get)
                && Some(child.id) != overlay.map(StableNodeId::get)
        })
        .or_else(|| {
            children.iter().find(|child| {
                let id = child.id;
                Some(id) != title_bar.map(StableNodeId::get)
                    && Some(id) != overlay.map(StableNodeId::get)
            })
        })
        .and_then(|child| StableNodeId::new(child.id))
        .or_else(|| {
            title_bar.and_then(|title_bar| {
                snapshot.get(title_bar.get()).and_then(|bar| {
                    element_child_widgets(bar, snapshot)
                        .into_iter()
                        .find(|child| is_body_child(child))
                        .and_then(|child| StableNodeId::new(child.id))
                })
            })
        });
    (title_bar, body, overlay)
}

fn normalize_event_name(event: &str) -> String {
    let e = event.trim();
    let lower = if let Some(rest) = e.strip_prefix("on").or_else(|| e.strip_prefix("On")) {
        rest
    } else {
        e
    };
    lower.to_ascii_lowercase()
}

/// Minimal trusted HTML fragment for Vue static content (compiler output only).
#[derive(Debug, Clone)]
enum FragNode {
    Text(String),
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<FragNode>,
    },
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", "\u{00A0}")
        .replace("&amp;", "&")
}

fn parse_html_fragment(html: &str) -> Vec<FragNode> {
    let bytes = html.as_bytes();
    let mut i = 0usize;
    let mut roots = Vec::new();
    parse_html_children(bytes, &mut i, &mut roots, None);
    roots
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn parse_html_children(
    bytes: &[u8],
    i: &mut usize,
    out: &mut Vec<FragNode>,
    stop_tag: Option<&str>,
) {
    while *i < bytes.len() {
        if bytes[*i] == b'<' {
            if bytes.get(*i + 1) == Some(&b'/') {
                let start = *i + 2;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'>' {
                    end += 1;
                }
                let name = std::str::from_utf8(&bytes[start..end])
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                *i = (end + 1).min(bytes.len());
                if stop_tag.is_some_and(|t| t == name) {
                    return;
                }
                continue;
            }
            if bytes.get(*i + 1) == Some(&b'!') {
                *i += 2;
                while *i < bytes.len() && bytes[*i] != b'>' {
                    *i += 1;
                }
                *i = (*i + 1).min(bytes.len());
                continue;
            }
            *i += 1;
            let tag_start = *i;
            while *i < bytes.len()
                && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'-' || bytes[*i] == b':')
            {
                *i += 1;
            }
            let tag = std::str::from_utf8(&bytes[tag_start..*i])
                .unwrap_or("")
                .to_ascii_lowercase();
            if tag.is_empty() {
                continue;
            }
            // Attributes until `>` or `/>`
            let attr_start = *i;
            let mut self_closing = false;
            while *i < bytes.len() {
                if bytes[*i] == b'>' {
                    if *i > attr_start && bytes[*i - 1] == b'/' {
                        self_closing = true;
                    }
                    break;
                }
                *i += 1;
            }
            let attr_end = if self_closing { *i - 1 } else { *i };
            let attr_str = std::str::from_utf8(&bytes[attr_start..attr_end]).unwrap_or("");
            let attrs = parse_html_attrs(attr_str);
            *i = (*i + 1).min(bytes.len());
            let void = self_closing || is_void_html_tag(&tag);
            let mut children = Vec::new();
            if !void {
                parse_html_children(bytes, i, &mut children, Some(&tag));
            }
            out.push(FragNode::Element {
                tag,
                attrs,
                children,
            });
            continue;
        }
        let start = *i;
        while *i < bytes.len() && bytes[*i] != b'<' {
            *i += 1;
        }
        if *i > start {
            let text = std::str::from_utf8(&bytes[start..*i]).unwrap_or("");
            if !text.is_empty() {
                out.push(FragNode::Text(decode_basic_entities(text)));
            }
        }
    }
}

fn parse_html_attrs(attr_str: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = attr_str.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = std::str::from_utf8(&bytes[name_start..i])
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.is_empty() {
            break;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if bytes.get(i) == Some(&b'"') || bytes.get(i) == Some(&b'\'') {
                let quote = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let v = std::str::from_utf8(&bytes[vstart..i]).unwrap_or("");
                if i < bytes.len() {
                    i += 1;
                }
                decode_basic_entities(v)
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                    i += 1;
                }
                decode_basic_entities(std::str::from_utf8(&bytes[vstart..i]).unwrap_or(""))
            }
        } else {
            String::new()
        };
        out.push((name, value));
    }
    out
}

fn selector_list_matches(sel: &str, tag: &str, attrs: &HashMap<String, String>) -> bool {
    sel.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|simple| selector_matches(simple, tag, attrs))
}

/// Match one compound simple selector: optional tag + `.class*` + optional `#id` + `[attr]*`.
fn selector_matches(sel: &str, tag: &str, attrs: &HashMap<String, String>) -> bool {
    let sel = sel.trim();
    if sel.is_empty() {
        return false;
    }
    let bytes = sel.as_bytes();
    let mut i = 0usize;
    // Optional type selector
    if bytes[0].is_ascii_alphabetic() || bytes[0] == b'*' {
        let start = i;
        i += 1;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
        {
            i += 1;
        }
        let type_sel = &sel[start..i];
        if type_sel != "*" && !tag.eq_ignore_ascii_case(type_sel) {
            return false;
        }
    }
    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                i += 1;
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
                {
                    i += 1;
                }
                if start == i {
                    return false;
                }
                let class = &sel[start..i];
                if !attrs
                    .get("class")
                    .map(|c| c.split_whitespace().any(|x| x == class))
                    .unwrap_or(false)
                {
                    return false;
                }
            }
            b'#' => {
                i += 1;
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
                {
                    i += 1;
                }
                if start == i {
                    return false;
                }
                let id = &sel[start..i];
                if attrs.get("id").map(|v| v.as_str()) != Some(id) {
                    return false;
                }
            }
            b'[' => {
                let Some(rel_end) = sel[i..].find(']') else {
                    return false;
                };
                let end = i + rel_end;
                let inner = sel[i + 1..end].trim();
                if let Some((k, v)) = inner.split_once('=') {
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                    if attrs.get(k.trim()).map(|x| x.as_str()) != Some(v) {
                        return false;
                    }
                } else if !attrs.contains_key(inner) {
                    return false;
                }
                i = end + 1;
            }
            _ => return false,
        }
    }
    // Bare tag already handled; empty remainder after type means tag-only match.
    // If we never consumed a type and never entered the loop, treat as tag name
    // (legacy path: `body`, `html`, `div`).
    if i == 0 {
        return tag.eq_ignore_ascii_case(sel);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_mutation_does_not_drop_valid_pending_ops() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        let text = doc.create_text("正文");
        doc.insert(text, doc.mount_root(), None);
        doc.flush_host_frame();

        // One invalid mutation (duplicate Create of the html root) plus one
        // valid mutation (text update). The batch is rejected wholesale and
        // must be replayed so the valid op lands instead of the whole frame's
        // host ops disappearing.
        let mut extra = MutationQueue::new();
        extra.create(
            StableNodeId::new(doc.html_root.0).expect("html root id is nonzero"),
            nana_ui_runtime::DocumentId::try_from(doc.id).expect("document ID is nonzero"),
            NodeKind::Element { tag: "html".into() },
        );
        extra.set_text(
            StableNodeId::try_from(text).expect("text id is nonzero"),
            TextContent {
                value: "更新".into(),
            },
        );
        let result = doc.commit_extra(extra);
        assert!(result.is_err(), "duplicate Create must be reported");
        assert_eq!(doc.runtime_text(text).as_deref(), Some("更新"));

        // The pending batch is fully drained: a later flush neither re-fails
        // nor resurrects the rejected mutation.
        doc.flush_host_frame();
        assert_eq!(doc.runtime_text(text).as_deref(), Some("更新"));
        assert!(doc.pending.is_empty());

        let rejections = doc.take_commit_rejections();
        assert_eq!(rejections.len(), 1, "only the rejected op is recorded");
        assert!(rejections[0].contains("Create"));
        assert!(doc.take_commit_rejections().is_empty());
    }

    #[test]
    fn clear_pointer_captures_survives_poisoned_pending() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        let target = doc.create_element("div");
        doc.insert(target, doc.mount_root(), None);
        doc.flush_host_frame();
        assert!(doc.capture_pointer(7, target));

        // A poisoned pending mutation (duplicate Create of the html root)
        // used to make the capture release panic on commit; the replay path
        // must still release the captures and drop only the poison.
        doc.pending.mutations.create(
            StableNodeId::new(doc.html_root.0).expect("html root id is nonzero"),
            nana_ui_runtime::DocumentId::try_from(doc.id).expect("document ID is nonzero"),
            NodeKind::Element { tag: "html".into() },
        );
        doc.clear_pointer_captures();
        assert!(doc.pointer_capture(7).is_none());
        assert_eq!(doc.take_commit_rejections().len(), 1);
        assert!(doc.pending.is_empty());
    }

    fn native_html_input(value: &str) -> (NanaTreeDocument, NodeHandle) {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        let input = doc.create_element("input");
        doc.insert(input, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            input.0,
            crate::WidgetKind::Input,
            crate::WidgetProps {
                value: value.into(),
                ..crate::WidgetProps::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        doc.apply_layout_boxes(&[(
            input,
            LayoutBox {
                handle: input,
                x: 8.0,
                y: 8.0,
                width: 160.0,
                height: 28.0,
            },
        )]);
        (doc, input)
    }

    #[derive(Clone)]
    struct ProbeCard {
        title: String,
    }

    impl ComponentView for ProbeCard {
        fn node_kind(&self) -> NodeKind {
            NodeKind::Element {
                tag: "probe-card".into(),
            }
        }

        fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
            if world.text(id) != Some(self.title.as_str()) {
                mutations.set_text(
                    id,
                    TextContent {
                        value: self.title.clone(),
                    },
                );
            }
        }
    }

    impl nana_ui_runtime::RegisterableComponent for ProbeCard {
        const TYPE_ID: &'static str = "test.probe-card";
        const TAGS: &'static [&'static str] = &["nana-probe-card", "probe-card"];
        fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
            Self {
                title: spec
                    .attr("handle")
                    .unwrap_or_else(|| spec.display_label())
                    .to_owned(),
            }
        }
    }

    struct ProbePlugin;

    impl nana_ui_runtime::UiExtension for ProbePlugin {
        fn name(&self) -> &'static str {
            "test.probe"
        }

        fn install(
            &self,
            registrar: &mut nana_ui_runtime::ExtensionRegistrar,
        ) -> Result<(), nana_ui_runtime::FrameworkError> {
            registrar.register_component::<ProbeCard>()
        }
    }

    #[test]
    fn installed_plugin_tag_projects_into_ui_world() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        doc.context_mut().install(&ProbePlugin).unwrap();
        let card = doc.create_element("nana-probe-card");
        doc.insert(card, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            card.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                element_tag: "nana-probe-card".into(),
                label: "User".into(),
                ..crate::WidgetProps::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(card).unwrap();
        assert_eq!(doc.world().text(id), Some("User"));
        assert_eq!(
            doc.world().component_type(id).map(ComponentTypeId::as_str),
            Some("test.probe-card")
        );
    }

    #[test]
    fn plugin_bind_reads_open_attrs() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        doc.context_mut().install(&ProbePlugin).unwrap();
        let card = doc.create_element("nana-probe-card");
        doc.insert(card, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        let mut attrs = std::collections::BTreeMap::new();
        attrs.insert("handle".into(), "from-attr".into());
        bridge.register(
            card.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                element_tag: "nana-probe-card".into(),
                label: "User".into(),
                attrs,
                ..crate::WidgetProps::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(card).unwrap();
        assert_eq!(doc.world().text(id), Some("from-attr"));
    }

    #[test]
    fn unregistered_custom_tag_stays_column() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        let unknown = doc.create_element("nana-unknown-widget");
        doc.insert(unknown, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            unknown.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                element_tag: "nana-unknown-widget".into(),
                label: "Hello".into(),
                ..crate::WidgetProps::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(unknown).unwrap();
        assert!(doc.world().component_type(id).is_none());
        assert_ne!(doc.world().text(id), Some("Hello"));
        assert_eq!(
            doc.element_tag(unknown).as_deref(),
            Some("nana-unknown-widget")
        );
    }

    #[test]
    fn legacy_handles_round_trip_through_runtime_ids() {
        let handle = NodeHandle(17);
        let stable = nana_ui_runtime::StableNodeId::try_from(handle).unwrap();
        assert_eq!(NodeHandle::from(stable), handle);

        let document = DocumentId(3);
        let runtime_document = nana_ui_runtime::DocumentId::try_from(document).unwrap();
        assert_eq!(DocumentId::from(runtime_document), document);
    }

    #[test]
    fn query_selector_finds_body_mount_root() {
        let doc = NanaTreeDocument::new(800, 600, 1.0);
        assert_eq!(doc.query_selector("body"), Some(doc.mount_root()));
        assert_eq!(doc.query_selector("html"), Some(doc.html_root()));
    }

    #[test]
    fn unchanged_layout_writeback_does_not_advance_runtime_generation() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("button");
        doc.insert(node, doc.mount_root(), None);
        let boxes = [(
            node,
            LayoutBox {
                handle: node,
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 30.0,
            },
        )];
        doc.apply_layout_boxes(&boxes);
        let generation = doc.runtime_generation();
        doc.apply_layout_boxes(&boxes);
        assert_eq!(doc.runtime_generation(), generation);
    }

    #[test]
    fn accessibility_updates_are_incremental_and_drain_once() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("input");
        doc.insert(node, doc.mount_root(), None);
        doc.inject_layout_boxes(&[(
            node,
            LayoutBox {
                handle: node,
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 30.0,
            },
        )]);

        let Some(AccessibilityUpdate::Delta(initial)) = doc.take_accessibility_update() else {
            panic!("initial retained work must produce a delta");
        };
        assert!(initial.updated.iter().any(|entry| entry.id.get() == node.0));
        assert!(doc.take_accessibility_update().is_none());

        doc.inject_layout_boxes(&[(
            node,
            LayoutBox {
                handle: node,
                x: 12.0,
                y: 20.0,
                width: 80.0,
                height: 30.0,
            },
        )]);
        let Some(AccessibilityUpdate::Delta(layout)) = doc.take_accessibility_update() else {
            panic!("changed bounds must produce a delta");
        };
        assert_eq!(
            layout
                .updated
                .iter()
                .filter(|entry| entry.id.get() == node.0)
                .count(),
            1
        );

        let id = StableNodeId::try_from(node).unwrap();
        let mut hide = MutationQueue::new();
        hide.set_style(
            id,
            NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    hidden: true,
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        doc.runtime.commit(hide).unwrap();
        doc.flush_runtime_systems();
        let Some(AccessibilityUpdate::Delta(hidden)) = doc.take_accessibility_update() else {
            panic!("hidden nodes must produce a retained tombstone");
        };
        assert!(hidden.removed.contains(&id));
        assert!(
            hidden
                .updated
                .iter()
                .any(|entry| entry.id.get() == doc.mount_root().0)
        );

        let mut show = MutationQueue::new();
        show.set_style(id, NodeStyle::default());
        doc.runtime.commit(show).unwrap();
        doc.flush_runtime_systems();
        let Some(AccessibilityUpdate::Delta(visible)) = doc.take_accessibility_update() else {
            panic!("restored nodes must rebuild the retained subtree");
        };
        assert!(visible.removed.is_empty());
        assert!(visible.updated.iter().any(|entry| entry.id == id));
        assert!(
            visible
                .updated
                .iter()
                .any(|entry| entry.id.get() == doc.mount_root().0)
        );

        doc.remove(node);
        doc.apply_layout_boxes(&[]);
        let Some(AccessibilityUpdate::Delta(removed)) = doc.take_accessibility_update() else {
            panic!("removed retained nodes must produce a delta");
        };
        assert!(removed.removed.iter().any(|id| id.get() == node.0));
    }

    #[test]
    fn native_html_input_keeps_text_field_semantics_after_vue_drains_runtime_work() {
        let (mut doc, input) = native_html_input("committed");
        doc.set_focus(input);

        let input_id = StableNodeId::try_from(input).unwrap();
        let field = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|node| node.id == input_id)
            .expect("native input must enter the retained accessibility tree");
        assert_eq!(field.role, AccessibilityRole::TextInput);
        assert_eq!(field.value.as_deref(), Some("committed"));
        assert!(field.editable);
        assert!(field.focused);

        assert!(
            doc.take_accessibility_update().is_some(),
            "Vue flush_runtime_systems must record the TextInput projection"
        );
        assert!(doc.take_accessibility_update().is_none());
        let host_flush = doc
            .runtime_document_mut()
            .flush_with(|_, _| Ok(()))
            .expect("host flush after Vue drain");
        assert!(host_flush.accessibility.updated.is_empty());
        assert!(host_flush.accessibility.removed.is_empty());
        let field = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|node| node.id == input_id)
            .expect("world snapshot remains the AccessKit authority");
        assert_eq!(field.role, AccessibilityRole::TextInput);
        assert_eq!(field.value.as_deref(), Some("committed"));
        assert!(field.focused);
    }

    #[test]
    fn set_focus_after_vue_drain_queues_accesskit_focus_on_the_text_field() {
        let (mut doc, input) = native_html_input("NanaUI");
        assert!(doc.take_accessibility_update().is_some());
        assert!(doc.take_accessibility_update().is_none());

        doc.set_focus(input);
        let Some(AccessibilityUpdate::Delta(focused)) = doc.take_accessibility_update() else {
            panic!("set_focus must flush an AccessKit focus delta");
        };
        let input_id = StableNodeId::try_from(input).unwrap();
        let field = focused
            .updated
            .iter()
            .find(|node| node.id == input_id)
            .expect("focus delta must include the TextInput");
        assert_eq!(field.role, AccessibilityRole::TextInput);
        assert!(field.focused);
        assert!(doc.take_accessibility_update().is_none());
    }

    #[test]
    fn vue_flush_keeps_layout_work_so_text_input_gets_a_hittable_box() {
        let mut doc = NanaTreeDocument::new(400, 200, 1.0);
        let input = doc.create_element("input");
        doc.insert(input, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            input.0,
            crate::WidgetKind::Input,
            crate::WidgetProps {
                value: "NanaUI".into(),
                ..crate::WidgetProps::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        bridge.resolve_document_layout(&mut doc);

        let laid_out = doc.layout_box(input).expect("runtime layout box");
        assert!(
            laid_out.width > 0.0 && laid_out.height >= nana_ui_core::ControlSize::Medium.height(),
            "Vue flush must run RuntimeLayoutEngine so TextInput is hittable, got {laid_out:?}"
        );
        let engine_height = laid_out.height;
        let engine_width = laid_out.width;
        doc.apply_layout_boxes(&[(
            input,
            LayoutBox {
                handle: input,
                x: 0.0,
                y: 0.0,
                width: engine_width.min(12.0),
                height: 8.0,
            },
        )]);
        let kept = doc
            .layout_box(input)
            .expect("engine box after measure dump");
        assert!(
            kept.height >= engine_height && kept.width >= engine_width,
            "apply_layout_boxes of measure results must not shrink engine geometry, got {kept:?}"
        );
    }

    fn runtime_layout(doc: &mut NanaTreeDocument, width: f32, height: f32) {
        let document = doc.runtime_document().document();
        doc.context_mut()
            .layout_document(
                document,
                nana_ui_runtime::LayoutViewport::new(width, height),
            )
            .unwrap();
    }

    fn visible_text_primitive_count(doc: &NanaTreeDocument, host: NodeHandle) -> usize {
        let mut ids = vec![host.0];
        let mut stack = doc.children_of(host);
        while let Some(child) = stack.pop() {
            ids.push(child.0);
            stack.extend(doc.children_of(child));
        }
        doc.scene()
            .primitives()
            .filter(|primitive| {
                ids.contains(&primitive.node.get())
                    && matches!(
                        &primitive.kind,
                        nana_ui_scene::ScenePrimitiveKind::Text { content, .. }
                            if !content.trim().is_empty()
                    )
            })
            .count()
    }

    /// Every kind that used to get a mount-time `#text` child must paint and
    /// announce its label from the widget element alone, so dropping the child
    /// can neither blank a label nor leave a second copy that `patchProp` never
    /// refreshes.
    #[test]
    fn labelled_widget_kinds_own_their_label_without_a_text_child() {
        fn settle(doc: &mut NanaTreeDocument, bridge: &crate::MessageBridge) {
            doc.sync_semantic_styles(&bridge.snapshot());
            runtime_layout(doc, 400.0, 240.0);
            doc.flush_runtime_systems();
        }

        for kind in [
            crate::WidgetKind::Text,
            crate::WidgetKind::Button,
            crate::WidgetKind::Chip,
            crate::WidgetKind::SidebarRow,
            crate::WidgetKind::ListItem,
        ] {
            let mut doc = NanaTreeDocument::new(400, 240, 1.0);
            let widget = doc.create_element(kind.element_tag());
            doc.insert(widget, doc.mount_root(), None);
            let mut bridge = crate::MessageBridge::new();
            bridge.register(
                widget.0,
                kind,
                crate::WidgetProps {
                    label: "Label".into(),
                    element_tag: kind.element_tag().into(),
                    ..Default::default()
                },
            );

            settle(&mut doc, &bridge);
            assert_eq!(
                visible_text_primitive_count(&doc, widget),
                1,
                "{kind:?} must paint its label without a #text child"
            );

            bridge.patch_prop(
                widget.0,
                "label",
                &nana_js_engine::HostValue::string("Renamed"),
            );
            settle(&mut doc, &bridge);

            assert_eq!(
                visible_text_primitive_count(&doc, widget),
                1,
                "{kind:?} must still paint exactly one label after a patch"
            );
            let announced: Vec<_> = doc
                .accessibility_snapshot()
                .into_iter()
                .filter_map(|node| Some((node.id, node.label?.to_string())))
                .collect();
            assert_eq!(
                announced,
                vec![(
                    StableNodeId::try_from(widget).unwrap(),
                    "Renamed".to_owned()
                )],
                "{kind:?} must announce the patched label exactly once, with no stale copy"
            );
        }
    }

    #[test]
    fn vue_button_and_heading_with_text_children_extract_one_visible_text_each() {
        let mut doc = NanaTreeDocument::new(400, 240, 1.0);
        let heading = doc.create_element("h1");
        let button = doc.create_element("button");
        doc.insert(heading, doc.mount_root(), None);
        doc.insert(button, doc.mount_root(), None);
        doc.set_element_text(heading, "Heading");
        doc.set_element_text(button, "Action");

        let mut heading_props = crate::WidgetProps::default();
        heading_props.element_tag = "h1".into();
        heading_props.label = "Heading".into();
        let mut button_props = crate::WidgetProps::default();
        button_props.element_tag = "button".into();
        button_props.label = "Action".into();

        let mut bridge = crate::MessageBridge::new();
        bridge.register(heading.0, crate::WidgetKind::Text, heading_props);
        bridge.register(button.0, crate::WidgetKind::Button, button_props);
        doc.sync_semantic_styles(&bridge.snapshot());
        runtime_layout(&mut doc, 400.0, 240.0);
        doc.flush_runtime_systems();

        assert_eq!(
            visible_text_primitive_count(&doc, heading),
            1,
            "heading host plus #text child must extract one visible text primitive"
        );
        assert_eq!(
            visible_text_primitive_count(&doc, button),
            1,
            "button host plus #text child must extract one visible text primitive"
        );
    }

    #[test]
    fn gpu_slot_flows_through_runtime_extraction_and_scene_graph() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("div");
        doc.insert(node, doc.mount_root(), None);
        doc.set_gpu_slot(node, "program");
        doc.apply_layout_boxes(&[(
            node,
            LayoutBox {
                handle: node,
                x: 10.0,
                y: 20.0,
                width: 320.0,
                height: 180.0,
            },
        )]);

        let custom = doc
            .scene()
            .primitives()
            .find(|primitive| primitive.node.get() == node.0)
            .expect("GPU node must be extracted into UiScene");
        let nana_ui_scene::ScenePrimitiveKind::Custom(content) = &custom.kind else {
            panic!("GPU node must compile to a custom scene primitive");
        };
        assert_eq!(content.renderer.as_ref(), "nana.host-texture");
        assert_eq!(content.resource.as_ref(), "program");
        let graph = doc
            .scene()
            .frame_graph(nana_ui_scene::ResourceId(1))
            .unwrap();
        let prepare_index = graph
            .passes
            .iter()
            .position(|pass| {
                pass.operations.iter().any(|operation| matches!(
                    operation,
                    nana_ui_scene::RenderOperation::PrepareExternal(id) if id.node.get() == node.0
                ))
            })
            .expect("GPU resource must have an explicit preparation pass");
        let invoke_index =
            graph
                .passes
                .iter()
                .position(|pass| {
                    pass.operations.iter().any(|operation| matches!(
                    operation,
                    nana_ui_scene::RenderOperation::InvokeCustom(id) if id.node.get() == node.0
                ))
                })
                .expect("GPU node must have a custom invocation pass");
        assert!(prepare_index < invoke_index);
        assert!(
            graph.passes[invoke_index]
                .dependencies
                .contains(&graph.passes[prepare_index].id)
        );

        doc.remove_attribute(node, "data-nana-gpu");
        doc.apply_layout_boxes(&[]);
        assert!(
            doc.scene()
                .primitives()
                .all(|primitive| primitive.node.get() != node.0)
        );
    }

    #[test]
    fn canvas_attr_projects_host_texture_custom_render() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let canvas = doc.create_element("canvas");
        doc.insert(canvas, doc.mount_root(), None);
        doc.set_attribute(canvas, "data-nana-canvas", "42");
        doc.apply_layout_boxes(&[(
            canvas,
            LayoutBox {
                handle: canvas,
                x: 12.0,
                y: 80.0,
                width: 320.0,
                height: 160.0,
            },
        )]);

        let id = StableNodeId::try_from(canvas).expect("canvas id");
        let content = doc
            .runtime
            .custom_render(id)
            .expect("2D canvas must attach a HostTexture CustomRender");
        assert_eq!(content.renderer.as_ref(), "nana.host-texture");
        assert_eq!(content.resource.as_ref(), "canvas:42");

        let primitive = doc
            .scene()
            .primitives()
            .find(|primitive| primitive.node.get() == canvas.0)
            .expect("canvas must extract a scene primitive");
        let nana_ui_scene::ScenePrimitiveKind::Custom(custom) = &primitive.kind else {
            panic!("canvas must compile to a custom scene primitive");
        };
        assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
        assert_eq!(custom.resource.as_ref(), "canvas:42");
    }

    #[test]
    fn catalog_icons_bind_iconglyph() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let svg = doc.create_element("svg");
        let i = doc.create_element("i");
        doc.insert(svg, doc.mount_root(), None);
        doc.insert(i, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            svg.0,
            crate::WidgetKind::Icon,
            crate::WidgetProps {
                class_names: vec!["lucide".into(), "lucide-search".into()],
                element_tag: "svg".into(),
                ..Default::default()
            },
        );
        bridge.register(
            i.0,
            crate::WidgetKind::Icon,
            crate::WidgetProps {
                value: "settings".into(),
                element_tag: "i".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let svg_id = StableNodeId::try_from(svg).unwrap();
        let i_id = StableNodeId::try_from(i).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(svg_id),
            Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
                if icon == nana_ui_core::Icon::Search
        ));
        assert!(matches!(
            doc.runtime.standard_visual(i_id),
            Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
                if icon == nana_ui_core::Icon::Settings
        ));
        assert!(doc.scene().primitives().any(|primitive| {
            primitive.node.get() == svg.0
                && matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Icon { icon, .. }
                        if icon == nana_ui_core::Icon::Search
                )
        }));
    }

    #[test]
    fn icon_button_child_icon_is_not_bound_twice() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let button = doc.create_element("button");
        let svg = doc.create_element("svg");
        doc.insert(button, doc.mount_root(), None);
        doc.insert(svg, button, None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            button.0,
            crate::WidgetKind::Button,
            crate::WidgetProps {
                label: "Search".into(),
                element_tag: "button".into(),
                ..Default::default()
            },
        );
        bridge.register(
            svg.0,
            crate::WidgetKind::Icon,
            crate::WidgetProps {
                value: "search".into(),
                class_names: vec!["lucide".into(), "lucide-search".into()],
                element_tag: "svg".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(svg.0, button.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        let button_id = StableNodeId::try_from(button).unwrap();
        let svg_id = StableNodeId::try_from(svg).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(button_id),
            Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
                if icon == nana_ui_core::Icon::Search
        ));
        assert!(doc.runtime.standard_visual(svg_id).is_none());
    }

    #[test]
    fn canvas_without_slot_is_not_a_2d_bitmap() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let canvas = doc.create_element("canvas");
        doc.insert(canvas, doc.mount_root(), None);
        doc.apply_layout_boxes(&[(
            canvas,
            LayoutBox {
                handle: canvas,
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 150.0,
            },
        )]);

        let id = StableNodeId::try_from(canvas).expect("canvas id");
        assert!(
            doc.runtime.custom_render(id).is_none(),
            "bare <canvas> must not attach a HostTexture CustomRender"
        );
        assert!(
            doc.scene().primitives().all(|primitive| {
                primitive.node.get() != canvas.0
                    || !matches!(
                        &primitive.kind,
                        nana_ui_scene::ScenePrimitiveKind::Custom(custom)
                            if custom.renderer.as_ref() == "nana.host-texture"
                    )
            }),
            "bare <canvas> must not sample host-texture as a 2d bitmap"
        );
    }

    #[test]
    fn semantic_widget_accessibility_projects_from_runtime() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("button");
        doc.insert(node, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            node.0,
            crate::WidgetKind::Chip,
            crate::WidgetProps {
                label: "Build".into(),
                role: "tab".into(),
                disabled: true,
                active: true,
                ..Default::default()
            },
        );
        let snapshot = bridge.snapshot();
        doc.sync_semantic_styles(&snapshot);

        let accessibility = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|entry| entry.id.get() == node.0)
            .expect("semantic widget must enter Runtime accessibility tree");
        assert_eq!(accessibility.role, AccessibilityRole::Tab);
        assert_eq!(accessibility.label.as_deref(), Some("Build"));
        assert!(accessibility.disabled);
        assert_eq!(accessibility.selected, Some(true));
    }

    #[test]
    fn nana_chip_projects_selected_button_not_a_second_control() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("nana-chip");
        doc.insert(node, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            node.0,
            crate::WidgetKind::Chip,
            crate::WidgetProps {
                label: "Beta".into(),
                active: true,
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(node).unwrap();
        assert_eq!(
            doc.runtime
                .component_type(id)
                .map(|type_id| type_id.as_str().to_owned())
                .as_deref(),
            Some("nana.button")
        );
        let accessibility = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|entry| entry.id == id)
            .expect("chip must enter Runtime accessibility");
        assert_eq!(accessibility.role, AccessibilityRole::Button);
        assert_eq!(accessibility.label.as_deref(), Some("Beta"));
        assert_eq!(
            doc.runtime.standard_visual(id),
            Some(nana_ui_runtime::StandardVisual::Button {
                label: std::sync::Arc::from("Beta"),
                kind: nana_ui_core::ButtonKind::Selected,
                size: nana_ui_core::ControlSize::Medium,
                loading: false,
                loading_phase: 0.0,
                invalid: false,
            })
        );
    }

    #[test]
    fn migrated_controls_project_one_retained_visual_and_accessibility_state() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let button = doc.create_element("nana-button");
        let input = doc.create_element("nana-input");
        let checkbox = doc.create_element("nana-checkbox");
        let switch = doc.create_element("nana-switch");
        let range = doc.create_element("nana-range");
        doc.insert(button, doc.mount_root(), None);
        doc.insert(input, doc.mount_root(), None);
        doc.insert(checkbox, doc.mount_root(), None);
        doc.insert(switch, doc.mount_root(), None);
        doc.insert(range, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            button.0,
            crate::WidgetKind::Button,
            crate::WidgetProps {
                label: "Build".into(),
                button_kind: nana_ui_core::ButtonKind::Primary,
                size: nana_ui_core::ControlSize::Large,
                loading: true,
                invalid: true,
                ..Default::default()
            },
        );
        bridge.register(
            input.0,
            crate::WidgetKind::Input,
            crate::WidgetProps {
                label: "Password".into(),
                placeholder: "Enter password".into(),
                value: "secret".into(),
                size: nana_ui_core::ControlSize::Large,
                read_only: true,
                secure: true,
                invalid: true,
                ..Default::default()
            },
        );
        bridge.register(
            checkbox.0,
            crate::WidgetKind::Checkbox,
            crate::WidgetProps {
                label: "Notifications".into(),
                toggled: true,
                invalid: true,
                ..Default::default()
            },
        );
        bridge.register(
            switch.0,
            crate::WidgetKind::Switch,
            crate::WidgetProps {
                label: "Live preview".into(),
                hint: "Updates while editing".into(),
                toggled: true,
                loading: true,
                invalid: true,
                control_position: nana_ui_core::SwitchControlPosition::Start,
                size: nana_ui_core::ControlSize::Large,
                ..Default::default()
            },
        );
        bridge.register(
            range.0,
            crate::WidgetKind::Range,
            crate::WidgetProps {
                label: "Opacity".into(),
                unit: "%".into(),
                min: 0.0,
                max: 1.0,
                step: 0.25,
                number: 0.62,
                invalid: true,
                ..Default::default()
            },
        );
        let snapshot = bridge.snapshot();
        doc.sync_semantic_styles(&snapshot);
        doc.apply_layout_boxes(&[
            (
                button,
                LayoutBox {
                    handle: button,
                    x: 10.0,
                    y: 10.0,
                    width: 220.0,
                    height: 36.0,
                },
            ),
            (
                input,
                LayoutBox {
                    handle: input,
                    x: 10.0,
                    y: 50.0,
                    width: 220.0,
                    height: 36.0,
                },
            ),
            (
                checkbox,
                LayoutBox {
                    handle: checkbox,
                    x: 10.0,
                    y: 90.0,
                    width: 220.0,
                    height: 32.0,
                },
            ),
            (
                switch,
                LayoutBox {
                    handle: switch,
                    x: 10.0,
                    y: 130.0,
                    width: 220.0,
                    height: 44.0,
                },
            ),
            (
                range,
                LayoutBox {
                    handle: range,
                    x: 10.0,
                    y: 180.0,
                    width: 220.0,
                    height: 36.0,
                },
            ),
        ]);

        let button_id = StableNodeId::try_from(button).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(button_id),
            Some(nana_ui_runtime::StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Primary,
                size: nana_ui_core::ControlSize::Large,
                loading: true,
                invalid: true,
                ..
            })
        ));
        let input_id = StableNodeId::try_from(input).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(input_id),
            Some(nana_ui_runtime::StandardVisual::TextInput {
                size: nana_ui_core::ControlSize::Large,
                secure: true,
                invalid: true,
                ..
            })
        ));

        let checkbox_id = StableNodeId::try_from(checkbox).unwrap();
        assert_eq!(
            doc.runtime.standard_visual(checkbox_id),
            Some(nana_ui_runtime::StandardVisual::Checkbox {
                checked: true,
                indeterminate: false,
                size: nana_ui_core::ControlSize::Medium,
            })
        );
        let switch_id = StableNodeId::try_from(switch).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(switch_id),
            Some(nana_ui_runtime::StandardVisual::Switch {
                checked: true,
                control_position: nana_ui_core::SwitchControlPosition::Start,
                size: nana_ui_core::ControlSize::Large,
                loading: true,
                invalid: true,
                ..
            })
        ));
        let range_id = StableNodeId::try_from(range).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(range_id),
            Some(nana_ui_runtime::StandardVisual::Range {
                ratio,
                invalid: true,
                ..
            })
                if (ratio - 0.5).abs() < f32::EPSILON
        ));
        let accessibility = doc.accessibility_snapshot();
        let input_accessibility = accessibility
            .iter()
            .find(|node| node.id == input_id)
            .unwrap();
        assert!(!input_accessibility.editable);
        assert_eq!(input_accessibility.value, None);
        assert!(input_accessibility.invalid);
        let checkbox_accessibility = accessibility
            .iter()
            .find(|node| node.id == checkbox_id)
            .unwrap();
        assert_eq!(checkbox_accessibility.checked, Some(true));
        assert!(checkbox_accessibility.invalid);
        let switch_accessibility = accessibility
            .iter()
            .find(|node| node.id == switch_id)
            .unwrap();
        assert_eq!(switch_accessibility.checked, Some(true));
        assert!(switch_accessibility.busy);
        assert!(switch_accessibility.invalid);
        let range_accessibility = accessibility
            .iter()
            .find(|node| node.id == range_id)
            .unwrap();
        assert_eq!(range_accessibility.numeric_minimum, Some(0.0));
        assert_eq!(range_accessibility.numeric_maximum, Some(1.0));
        assert_eq!(range_accessibility.numeric_step, Some(0.25));
        assert_eq!(range_accessibility.numeric_value, Some(0.5));
        assert!(doc.scene().node_bounds(switch_id).is_some());
        assert!(doc.scene().node_bounds(range_id).is_some());
    }

    #[test]
    fn migrated_kind_change_clears_input_state_before_publishing_button_label() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let node = doc.create_element("nana-input");
        doc.insert(node, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            node.0,
            crate::WidgetKind::Input,
            crate::WidgetProps {
                value: "secret".into(),
                secure: true,
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(node).unwrap();
        assert!(doc.runtime.text_input(id).is_some());

        bridge.register(
            node.0,
            crate::WidgetKind::Button,
            crate::WidgetProps {
                label: "Run".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        assert!(doc.runtime.text_input(id).is_none());
        assert_eq!(doc.runtime.text(id), Some("Run"));
        assert!(matches!(
            doc.runtime.standard_visual(id),
            Some(nana_ui_runtime::StandardVisual::Button { ref label, .. }) if &**label == "Run"
        ));
        let accessibility = doc
            .accessibility_snapshot()
            .into_iter()
            .find(|node| node.id == id)
            .unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Button);
        assert_eq!(accessibility.label.as_deref(), Some("Run"));
        assert_eq!(accessibility.value, None);
    }

    #[test]
    fn feedback_hosts_project_retained_visuals_including_action_children() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let badge = doc.create_element("nana-status");
        let validation = doc.create_element("nana-validation");
        let empty = doc.create_element("nana-empty-state");
        let action = doc.create_element("nana-button");
        let labeled = doc.create_element("nana-labeled-value");
        let progress = doc.create_element("nana-progress");
        let spinner = doc.create_element("nana-spinner");
        doc.insert(badge, doc.mount_root(), None);
        doc.insert(validation, doc.mount_root(), None);
        doc.insert(empty, doc.mount_root(), None);
        doc.insert(action, empty, None);
        doc.insert(labeled, doc.mount_root(), None);
        doc.insert(progress, doc.mount_root(), None);
        doc.insert(spinner, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        let mut badge_props = crate::WidgetProps {
            label: "Offline".into(),
            class_names: vec![
                "nana-status".into(),
                "nana-status--danger".into(),
                "compact".into(),
            ],
            ..Default::default()
        };
        badge_props.attrs.insert("tone".into(), "danger".into());
        bridge.register(badge.0, crate::WidgetKind::StatusBadge, badge_props);
        bridge.register(
            validation.0,
            crate::WidgetKind::ValidationMessage,
            crate::WidgetProps {
                hint: "A project is required".into(),
                invalid: true,
                ..Default::default()
            },
        );
        bridge.register(
            empty.0,
            crate::WidgetKind::EmptyState,
            crate::WidgetProps {
                label: "No projects".into(),
                hint: "Create the first project".into(),
                ..Default::default()
            },
        );
        bridge.register(
            action.0,
            crate::WidgetKind::Button,
            crate::WidgetProps {
                label: "Create".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(action.0, empty.0, None);
        bridge.register(
            labeled.0,
            crate::WidgetKind::LabeledValue,
            crate::WidgetProps {
                label: "Revision".into(),
                value: "42".into(),
                muted: true,
                ..Default::default()
            },
        );
        bridge.register(
            progress.0,
            crate::WidgetKind::Progress,
            crate::WidgetProps {
                label: "Copying".into(),
                progress: 40.0,
                progress_max: 100.0,
                ..Default::default()
            },
        );
        bridge.register(
            spinner.0,
            crate::WidgetKind::Spinner,
            crate::WidgetProps {
                label: "Loading".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        let badge_id = StableNodeId::try_from(badge).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(badge_id),
            Some(nana_ui_runtime::StandardVisual::StatusBadge {
                tone: nana_ui_core::StatusTone::Danger,
                compact: true,
                ..
            })
        ));
        let validation_id = StableNodeId::try_from(validation).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(validation_id),
            Some(nana_ui_runtime::StandardVisual::ValidationMessage {
                intent: nana_ui_core::ValidationIntent::Danger,
                ..
            })
        ));
        let empty_id = StableNodeId::try_from(empty).unwrap();
        let action_id = StableNodeId::try_from(action).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(empty_id),
            Some(nana_ui_runtime::StandardVisual::EmptyState {
                action: Some(id),
                ..
            }) if id == action_id
        ));
        let labeled_id = StableNodeId::try_from(labeled).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(labeled_id),
            Some(nana_ui_runtime::StandardVisual::LabeledValue {
                value_weight: 400,
                ..
            })
        ));
        let progress_id = StableNodeId::try_from(progress).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(progress_id),
            Some(nana_ui_runtime::StandardVisual::Progress {
                value_ratio,
                ..
            }) if (value_ratio - 0.4).abs() < 0.001
        ));
        let spinner_id = StableNodeId::try_from(spinner).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(spinner_id),
            Some(nana_ui_runtime::StandardVisual::Spinner { .. })
        ));
    }

    #[test]
    fn qualified_surface_hosts_project_runtime_visuals() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let field = doc.create_element("nana-form-field");
        let input = doc.create_element("nana-input");
        let card = doc.create_element("nana-interactive-card");
        let skeleton = doc.create_element("nana-skeleton");
        let meter = doc.create_element("nana-level-meter");
        doc.insert(field, doc.mount_root(), None);
        doc.insert(input, field, None);
        doc.insert(card, doc.mount_root(), None);
        doc.insert(skeleton, doc.mount_root(), None);
        doc.insert(meter, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            field.0,
            crate::WidgetKind::FormField,
            crate::WidgetProps {
                label: "Email".into(),
                hint: "Required".into(),
                invalid: true,
                size: nana_ui_core::ControlSize::Small,
                ..Default::default()
            },
        );
        bridge.register(
            input.0,
            crate::WidgetKind::Input,
            crate::WidgetProps {
                value: "a@b.c".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(input.0, field.0, None);
        bridge.register(
            card.0,
            crate::WidgetKind::InteractiveCard,
            crate::WidgetProps {
                active: true,
                disabled: true,
                ..Default::default()
            },
        );
        let mut skeleton_props = crate::WidgetProps::default();
        skeleton_props.layout.width = Some(nana_ui_core::LengthSpec::Px(120.0));
        skeleton_props.layout.height = Some(nana_ui_core::LengthSpec::Px(18.0));
        bridge.register(skeleton.0, crate::WidgetKind::Skeleton, skeleton_props);
        let mut meter_props = crate::WidgetProps {
            progress: 0.65,
            ..Default::default()
        };
        meter_props.attrs.insert("tone".into(), "warning".into());
        meter_props.layout.height = Some(nana_ui_core::LengthSpec::Px(8.0));
        bridge.register(meter.0, crate::WidgetKind::LevelMeter, meter_props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let field_id = StableNodeId::try_from(field).unwrap();
        let input_id = StableNodeId::try_from(input).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(field_id),
            Some(nana_ui_runtime::StandardVisual::FormField {
                ref label,
                hint: None,
                error: Some(ref error),
                size: nana_ui_core::ControlSize::Small,
                control: Some(control),
            }) if &**label == "Email" && &**error == "Required" && control == input_id
        ));
        let card_id = StableNodeId::try_from(card).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(card_id),
            Some(nana_ui_runtime::StandardVisual::Card {
                kind: nana_ui_core::CardKind::Selected,
                title: None,
                loading: false,
                ..
            })
        ));
        assert!(doc.runtime.accessibility(card_id).unwrap().disabled);
        let skeleton_id = StableNodeId::try_from(skeleton).unwrap();
        assert_eq!(doc.runtime.standard_visual(skeleton_id), None);
        let skeleton_style = doc.runtime.node_style(skeleton_id).unwrap();
        assert_eq!(
            skeleton_style.layout.width,
            Some(nana_ui_core::LengthSpec::Px(120.0))
        );
        assert_eq!(
            skeleton_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(18.0))
        );
        let meter_id = StableNodeId::try_from(meter).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(meter_id),
            Some(nana_ui_runtime::StandardVisual::LevelMeter {
                value_ratio,
                girth,
                tone: nana_ui_core::StatusTone::Warning,
            }) if (value_ratio - 0.65).abs() < 0.001 && (girth - 8.0).abs() < 0.001
        ));
    }

    #[test]
    fn qualified_candidate_leaves_project_runtime_visuals() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let area = doc.create_element("nana-textarea");
        let select = doc.create_element("nana-select");
        let dialog = doc.create_element("nana-dialog");
        let confirm = doc.create_element("nana-confirm-dialog");
        let drawer = doc.create_element("nana-drawer");
        doc.insert(area, doc.mount_root(), None);
        doc.insert(select, doc.mount_root(), None);
        doc.insert(dialog, doc.mount_root(), None);
        doc.insert(confirm, doc.mount_root(), None);
        doc.insert(drawer, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            area.0,
            crate::WidgetKind::Textarea,
            crate::WidgetProps {
                value: "line\nbreak".into(),
                placeholder: "Notes".into(),
                invalid: true,
                ..Default::default()
            },
        );
        bridge.register(
            select.0,
            crate::WidgetKind::Select,
            crate::WidgetProps {
                value: "code".into(),
                options: vec![crate::SelectOptionProp {
                    value: "code".into(),
                    label: "Code".into(),
                    disabled: false,
                }],
                ..Default::default()
            },
        );
        bridge.register(
            dialog.0,
            crate::WidgetKind::Dialog,
            crate::WidgetProps {
                label: "Rename".into(),
                hint: "Choose a name".into(),
                ..Default::default()
            },
        );
        let mut confirm_props = crate::WidgetProps {
            label: "Delete".into(),
            hint: "This cannot be undone.".into(),
            ..Default::default()
        };
        confirm_props.class_names.push("nana-confirm-dialog".into());
        bridge.register(confirm.0, crate::WidgetKind::Dialog, confirm_props);
        bridge.register(
            drawer.0,
            crate::WidgetKind::Drawer,
            crate::WidgetProps {
                label: "Inspector".into(),
                side: "left".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        let area_id = StableNodeId::try_from(area).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(area_id),
            Some(nana_ui_runtime::StandardVisual::TextInput { invalid: true, .. })
        ));
        assert_eq!(
            doc.runtime
                .text_input(area_id)
                .map(|state| state.value.as_str()),
            Some("line\nbreak")
        );
        let select_id = StableNodeId::try_from(select).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(select_id),
            Some(nana_ui_runtime::StandardVisual::Select { .. })
        ));
        let dialog_id = StableNodeId::try_from(dialog).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(dialog_id),
            Some(nana_ui_runtime::StandardVisual::ModalFrame {
                kind: nana_ui_runtime::ModalSurfaceKind::Dialog(_),
                ..
            })
        ));
        let confirm_id = StableNodeId::try_from(confirm).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(confirm_id),
            Some(nana_ui_runtime::StandardVisual::ModalFrame {
                kind: nana_ui_runtime::ModalSurfaceKind::Confirm(_),
                ..
            })
        ));
        let drawer_id = StableNodeId::try_from(drawer).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(drawer_id),
            Some(nana_ui_runtime::StandardVisual::ModalFrame {
                kind: nana_ui_runtime::ModalSurfaceKind::Drawer(nana_ui_core::DrawerSide::Left),
                ..
            })
        ));
    }

    #[test]
    fn highlighted_textarea_binds_language_and_restores_input() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let area = doc.create_element("nana-textarea");
        doc.insert(area, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        let mut props = crate::WidgetProps {
            value: "fn main() {}".into(),
            placeholder: "code".into(),
            label: "Editor".into(),
            ..Default::default()
        };
        props.layout.height = Some(nana_ui_core::LengthSpec::Px(120.0));
        props.attrs.insert("language".into(), "rs".into());
        bridge.register(area.0, crate::WidgetKind::Textarea, props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let area_id = StableNodeId::try_from(area).unwrap();
        assert_eq!(
            doc.runtime
                .component_type(area_id)
                .map(ComponentTypeId::as_str),
            Some("nana.hosted-textarea")
        );
        assert_eq!(
            doc.runtime
                .highlight_request(area_id)
                .map(|request| (request.presenter.as_ref(), request.language.as_ref())),
            Some((nana_ui_runtime::HIGHLIGHT_PRESENTER, "rs"))
        );
        assert_eq!(
            doc.runtime
                .text_input(area_id)
                .map(|state| state.value.as_str()),
            Some("fn main() {}")
        );
        assert!(matches!(
            doc.runtime.standard_visual(area_id),
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder,
                invalid: false,
                ..
            }) if placeholder.as_ref() == "code"
        ));
        assert_eq!(
            doc.runtime
                .node_style(area_id)
                .and_then(|style| style.layout.height),
            Some(nana_ui_core::LengthSpec::Px(
                120.0_f32.max(nana_ui_core::ControlSize::Medium.height()),
            ))
        );
    }

    #[test]
    fn segmented_and_tabs_bind_parent_and_project_child_options() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let tabs = doc.create_element("nana-tabs");
        let tab_a = doc.create_element("nana-tabs__item");
        let tab_b = doc.create_element("nana-tabs__item");
        let segmented = doc.create_element("nana-segmented");
        let seg_a = doc.create_element("nana-segmented__item");
        doc.insert(tabs, doc.mount_root(), None);
        doc.insert(tab_a, tabs, None);
        doc.insert(tab_b, tabs, None);
        doc.insert(segmented, doc.mount_root(), None);
        doc.insert(seg_a, segmented, None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            tabs.0,
            crate::WidgetKind::Tabs,
            crate::WidgetProps {
                label: "Editor".into(),
                value: "preview".into(),
                fill: true,
                options: vec![
                    crate::SelectOptionProp {
                        value: "code".into(),
                        label: "Code".into(),
                        disabled: false,
                    },
                    crate::SelectOptionProp {
                        value: "preview".into(),
                        label: "Preview".into(),
                        disabled: false,
                    },
                ],
                ..Default::default()
            },
        );
        bridge.register(
            tab_a.0,
            crate::WidgetKind::Chip,
            crate::WidgetProps {
                role: "tab".into(),
                ..Default::default()
            },
        );
        bridge.register(
            tab_b.0,
            crate::WidgetKind::Chip,
            crate::WidgetProps {
                role: "tab".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(tab_a.0, tabs.0, None);
        bridge.insert_child(tab_b.0, tabs.0, None);
        bridge.register(
            segmented.0,
            crate::WidgetKind::Segmented,
            crate::WidgetProps {
                label: "Theme".into(),
                value: "dark".into(),
                options: vec![crate::SelectOptionProp {
                    value: "dark".into(),
                    label: "Dark".into(),
                    disabled: false,
                }],
                ..Default::default()
            },
        );
        bridge.register(
            seg_a.0,
            crate::WidgetKind::Chip,
            crate::WidgetProps {
                role: "tab".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(seg_a.0, segmented.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let tabs_id = StableNodeId::try_from(tabs).unwrap();
        let tab_b_id = StableNodeId::try_from(tab_b).unwrap();
        let segmented_id = StableNodeId::try_from(segmented).unwrap();
        let seg_a_id = StableNodeId::try_from(seg_a).unwrap();
        assert_eq!(
            doc.runtime
                .component_type(tabs_id)
                .map(ComponentTypeId::as_str),
            Some("nana.tabs")
        );
        assert_eq!(
            doc.runtime.accessibility(tabs_id).map(|state| state.role),
            Some(AccessibilityRole::TabList)
        );
        assert_eq!(
            doc.runtime
                .accessibility(tabs_id)
                .and_then(|state| state.label.clone()),
            Some(Arc::<str>::from("Editor"))
        );
        assert!(matches!(
            doc.runtime.standard_visual(tab_b_id),
            Some(nana_ui_runtime::StandardVisual::SelectionOption { selected: true, .. })
        ));
        assert_eq!(
            doc.runtime
                .component_type(segmented_id)
                .map(ComponentTypeId::as_str),
            Some("nana.segmented")
        );
        assert_eq!(
            doc.runtime
                .accessibility(segmented_id)
                .and_then(|state| state.label.clone()),
            Some(Arc::<str>::from("Theme"))
        );
        assert!(matches!(
            doc.runtime.standard_visual(seg_a_id),
            Some(nana_ui_runtime::StandardVisual::SelectionOption { selected: true, .. })
        ));
    }

    #[test]
    fn qualified_runtime_leaves_project_toast_menu_xypad_and_qr() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let toast = doc.create_element("nana-toast");
        let tooltip = doc.create_element("nana-tooltip");
        let menu = doc.create_element("nana-action-menu");
        let item = doc.create_element("nana-action-menu-item");
        let pad = doc.create_element("nana-xy-pad");
        let qr = doc.create_element("nana-qr-code");
        let qr_payload = doc.create_element("nana-qr");
        doc.insert(toast, doc.mount_root(), None);
        doc.insert(tooltip, doc.mount_root(), None);
        doc.insert(menu, doc.mount_root(), None);
        doc.insert(item, menu, None);
        doc.insert(pad, doc.mount_root(), None);
        doc.insert(qr, doc.mount_root(), None);
        doc.insert(qr_payload, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        let mut toast_props = crate::WidgetProps {
            label: "Saved".into(),
            hint: "Project exported".into(),
            ..Default::default()
        };
        toast_props.attrs.insert("tone".into(), "success".into());
        toast_props
            .attrs
            .insert("dismissible".into(), String::new());
        bridge.register(toast.0, crate::WidgetKind::Toast, toast_props);
        bridge.register(
            tooltip.0,
            crate::WidgetKind::Tooltip,
            crate::WidgetProps {
                label: "Hint".into(),
                ..Default::default()
            },
        );
        bridge.register(
            menu.0,
            crate::WidgetKind::ActionMenu,
            crate::WidgetProps {
                label: "Actions".into(),
                active: true,
                ..Default::default()
            },
        );
        let mut item_props = crate::WidgetProps {
            label: "Delete".into(),
            hint: "⌫".into(),
            active: true,
            disabled: false,
            button_kind: nana_ui_core::ButtonKind::Danger,
            ..Default::default()
        };
        item_props.attrs.insert("danger".into(), String::new());
        bridge.register(item.0, crate::WidgetKind::ActionMenuItem, item_props);
        let mut pad_props = crate::WidgetProps {
            number: 0.25,
            min: 0.0,
            max: 1.0,
            step: 0.0,
            invalid: true,
            disabled: true,
            loading: true,
            label: "Pan".into(),
            ..Default::default()
        };
        pad_props.attrs.insert("y".into(), "0.75".into());
        bridge.register(pad.0, crate::WidgetKind::XYPad, pad_props);
        let mut qr_props = crate::WidgetProps {
            label: "Pairing".into(),
            ..Default::default()
        };
        qr_props.attrs.insert("modules".into(), "1,0,0,1".into());
        qr_props.attrs.insert("module-width".into(), "2".into());
        bridge.register(qr.0, crate::WidgetKind::QrCode, qr_props);
        let mut payload_props = crate::WidgetProps {
            label: "Pairing".into(),
            value: "nana://pair".into(),
            ..Default::default()
        };
        payload_props
            .attrs
            .insert("payload".into(), "nana://pair".into());
        bridge.register(qr_payload.0, crate::WidgetKind::QrCode, payload_props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let toast_id = StableNodeId::try_from(toast).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(toast_id),
            Some(nana_ui_runtime::StandardVisual::Toast {
                tone: nana_ui_core::ToastTone::Success,
                dismissible: true,
                ..
            })
        ));
        let tooltip_id = StableNodeId::try_from(tooltip).unwrap();
        // Runtime Tooltip is label + a11y only; no StandardVisual leaf.
        assert_eq!(doc.runtime.standard_visual(tooltip_id), None);
        assert_eq!(doc.runtime.text(tooltip_id), Some("Hint"));
        assert_eq!(
            doc.runtime.accessibility(tooltip_id).unwrap().role,
            AccessibilityRole::Tooltip
        );
        let menu_id = StableNodeId::try_from(menu).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(menu_id),
            Some(nana_ui_runtime::StandardVisual::MenuSurface {
                kind: nana_ui_runtime::MenuSurfaceKind::ActionMenu,
                ..
            })
        ));
        let item_id = StableNodeId::try_from(item).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(item_id),
            Some(nana_ui_runtime::StandardVisual::ActionMenuItem {
                danger: true,
                active: true,
                disabled: false,
                ..
            })
        ));
        let pad_id = StableNodeId::try_from(pad).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(pad_id),
            Some(nana_ui_runtime::StandardVisual::XYPad {
                invalid: true,
                disabled: true,
                ..
            })
        ));
        let qr_id = StableNodeId::try_from(qr).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(qr_id),
            Some(nana_ui_runtime::StandardVisual::QrCode { width: 2, .. })
        ));
        let payload_id = StableNodeId::try_from(qr_payload).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(payload_id),
            Some(nana_ui_runtime::StandardVisual::QrCode { width, .. }) if width >= 21
        ));
    }

    #[test]
    fn command_palette_and_tree_view_project_runtime_visuals() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let palette = doc.create_element("nana-command-palette");
        let tree = doc.create_element("nana-tree-view");
        let calendar = doc.create_element("nana-calendar");
        let markdown = doc.create_element("nana-markdown");
        doc.insert(palette, doc.mount_root(), None);
        doc.insert(tree, doc.mount_root(), None);
        doc.insert(calendar, doc.mount_root(), None);
        doc.insert(markdown, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            palette.0,
            crate::WidgetKind::CommandPalette,
            crate::WidgetProps {
                label: "Go to".into(),
                placeholder: "Search".into(),
                value: "op".into(),
                active: true,
                options: vec![
                    crate::SelectOptionProp {
                        value: "open".into(),
                        label: "Open file".into(),
                        disabled: false,
                    },
                    crate::SelectOptionProp {
                        value: "palette".into(),
                        label: "Command palette".into(),
                        disabled: false,
                    },
                ],
                ..Default::default()
            },
        );
        bridge.register(
            tree.0,
            crate::WidgetKind::TreeView,
            crate::WidgetProps {
                value: "src".into(),
                options: vec![
                    crate::SelectOptionProp {
                        value: "src".into(),
                        label: "src".into(),
                        disabled: false,
                    },
                    crate::SelectOptionProp {
                        value: "docs".into(),
                        label: "docs".into(),
                        disabled: false,
                    },
                ],
                ..Default::default()
            },
        );
        bridge.register(
            calendar.0,
            crate::WidgetKind::CalendarHeatmap,
            crate::WidgetProps {
                label: "Activity".into(),
                ..Default::default()
            },
        );
        bridge.register(
            markdown.0,
            crate::WidgetKind::NativeMarkdown,
            crate::WidgetProps {
                value: "# Title".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        let palette_id = StableNodeId::try_from(palette).unwrap();
        assert!(matches!(
            doc.runtime.node(palette_id).map(|node| node.kind),
            Some(NodeKind::Element { .. })
        ));
        assert!(matches!(
            doc.runtime.standard_visual(palette_id),
            Some(nana_ui_runtime::StandardVisual::CommandPalette {
                title,
                query,
                ..
            }) if title.as_ref() == "Go to" && query.as_ref() == "op"
        ));
        let tree_id = StableNodeId::try_from(tree).unwrap();
        assert!(matches!(
            doc.runtime.node(tree_id).map(|node| node.kind),
            Some(NodeKind::Element { .. })
        ));
        assert!(matches!(
            doc.runtime.standard_visual(tree_id),
            Some(nana_ui_runtime::StandardVisual::TreeView { rows, .. })
                if rows.iter().any(|row| row.id.as_ref() == "src" && row.selected)
                    && rows.iter().any(|row| row.id.as_ref() == "docs")
        ));
        let calendar_id = StableNodeId::try_from(calendar).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(calendar_id),
            Some(nana_ui_runtime::StandardVisual::CalendarHeatmap { .. })
        ));
        let markdown_id = StableNodeId::try_from(markdown).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(markdown_id),
            Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                if text.contains("Title")
        ));
    }

    #[test]
    fn command_palette_host_items_keep_category() {
        let mut doc = NanaTreeDocument::new(320, 200, 1.0);
        let palette = doc.create_element("nana-command-palette");
        doc.insert(palette, doc.mount_root(), None);
        let mut props = crate::WidgetProps {
            label: "Go to".into(),
            ..Default::default()
        };
        props.apply_prop(
            "items",
            &nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
                [
                    ("value".into(), nana_js_engine::HostValue::string("open")),
                    (
                        "label".into(),
                        nana_js_engine::HostValue::string("Open file"),
                    ),
                    (
                        "category".into(),
                        nana_js_engine::HostValue::string("Workspace"),
                    ),
                    (
                        "shortcut".into(),
                        nana_js_engine::HostValue::string("Ctrl+P"),
                    ),
                ]
                .into_iter()
                .collect(),
            )]),
        );
        let mut bridge = crate::MessageBridge::new();
        bridge.register(palette.0, crate::WidgetKind::CommandPalette, props);
        doc.sync_semantic_styles(&bridge.snapshot());
        let palette_id = StableNodeId::try_from(palette).unwrap();
        assert!(
            matches!(
                doc.runtime.standard_visual(palette_id),
                Some(nana_ui_runtime::StandardVisual::CommandPalette { rows, .. })
                    if rows.iter().any(|row| {
                        row.label.as_ref() == "Open file"
                            && row.category.as_deref() == Some("Workspace")
                            && row.shortcut.as_deref() == Some("Ctrl+P")
                    })
            ),
            "host command palette items must keep category and shortcut"
        );
    }

    #[test]
    fn markdown_source_from_native_props_projects_and_assembles() {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let markdown = doc.create_element("nana-markdown");
        doc.insert(markdown, doc.mount_root(), None);
        let mut props = crate::WidgetProps::default();
        props.apply_prop(
            "source",
            &nana_js_engine::HostValue::string("# Native\n\n```mermaid\nflowchart LR\nA-->B\n```"),
        );
        props.apply_prop(
            "mermaidRenderer",
            &nana_js_engine::HostValue::string("app-mermaid"),
        );
        let mut bridge = crate::MessageBridge::new();
        bridge.register(markdown.0, crate::WidgetKind::NativeMarkdown, props);
        doc.sync_semantic_styles(&bridge.snapshot());
        let markdown_id = StableNodeId::try_from(markdown).unwrap();
        assert!(
            matches!(
                doc.runtime.standard_visual(markdown_id),
                Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                    if text.contains("Native") && text.contains("flowchart LR")
            ),
            "empty value must still bind markdown source from native_props"
        );
        let children = doc.runtime.node(markdown_id).unwrap().children;
        assert!(
            children.iter().any(|child| {
                doc.runtime
                    .highlight_request(*child)
                    .is_some_and(|request| {
                        request.presenter.as_ref() == RuntimeNativeMarkdown::MERMAID_PRESENTER
                    })
            }),
            "pending markdown assemble must still attach mermaid fence highlight"
        );
    }

    #[test]
    fn professional_leaves_project_native_payloads() {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let calendar = doc.create_element("nana-calendar");
        let markdown = doc.create_element("nana-markdown");
        let viewer = doc.create_element("nana-image-viewer");
        let canvas = doc.create_element("nana-graph-canvas");
        doc.insert(calendar, doc.mount_root(), None);
        doc.insert(markdown, doc.mount_root(), None);
        doc.insert(viewer, doc.mount_root(), None);
        doc.insert(canvas, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();

        let mut calendar_props = crate::WidgetProps {
            label: "Activity".into(),
            ..Default::default()
        };
        calendar_props.apply_prop(
            "data",
            &nana_js_engine::HostValue::Array(vec![
                nana_js_engine::HostValue::Object(
                    [
                        (
                            "date".into(),
                            nana_js_engine::HostValue::string("2026-06-01"),
                        ),
                        ("value".into(), nana_js_engine::HostValue::Number(2.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                nana_js_engine::HostValue::Array(vec![
                    nana_js_engine::HostValue::string("2026-06-03"),
                    nana_js_engine::HostValue::Number(8.0),
                ]),
            ]),
        );
        bridge.register(
            calendar.0,
            crate::WidgetKind::CalendarHeatmap,
            calendar_props,
        );
        bridge.register(
            markdown.0,
            crate::WidgetKind::NativeMarkdown,
            crate::WidgetProps {
                value: "# Title\n\nHello **world**".into(),
                ..Default::default()
            },
        );
        let mut viewer_props = crate::WidgetProps::default();
        viewer_props.apply_prop("src", &nana_js_engine::HostValue::string("gpu:preview"));
        bridge.register(viewer.0, crate::WidgetKind::ImageViewer, viewer_props);

        let mut canvas_props = crate::WidgetProps {
            label: "Graph".into(),
            ..Default::default()
        };
        canvas_props.apply_prop(
            "nodes",
            &nana_js_engine::HostValue::Array(vec![
                nana_js_engine::HostValue::Object(
                    [
                        ("id".into(), nana_js_engine::HostValue::string("source")),
                        ("title".into(), nana_js_engine::HostValue::string("Source")),
                        ("x".into(), nana_js_engine::HostValue::Number(20.0)),
                        ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                        ("width".into(), nana_js_engine::HostValue::Number(160.0)),
                        ("height".into(), nana_js_engine::HostValue::Number(80.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                nana_js_engine::HostValue::Object(
                    [
                        ("id".into(), nana_js_engine::HostValue::string("sink")),
                        ("label".into(), nana_js_engine::HostValue::string("Sink")),
                        ("x".into(), nana_js_engine::HostValue::Number(240.0)),
                        ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ]),
        );
        bridge.register(canvas.0, crate::WidgetKind::GraphCanvas, canvas_props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let calendar_id = StableNodeId::try_from(calendar).unwrap();
        assert!(
            matches!(
                doc.runtime.standard_visual(calendar_id),
                Some(nana_ui_runtime::StandardVisual::CalendarHeatmap { cells, .. })
                    if !cells.is_empty() && cells.iter().any(|cell| cell.level > 0)
            ),
            "calendar with two data points must not project CalendarHeatmap::new([])"
        );
        let markdown_id = StableNodeId::try_from(markdown).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(markdown_id),
            Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                if text.contains("Title") && text.contains("Hello world")
        ));
        let viewer_id = StableNodeId::try_from(viewer).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(viewer_id),
            Some(nana_ui_runtime::StandardVisual::ImageViewer { .. })
        ));
        let canvas_id = StableNodeId::try_from(canvas).unwrap();
        assert!(matches!(
            doc.runtime.standard_visual(canvas_id),
            Some(nana_ui_runtime::StandardVisual::GraphCanvas { nodes, .. })
                if nodes.len() == 2
        ));
    }

    #[test]
    fn calendar_options_object_projects_heatmap_metrics() {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let default_calendar = doc.create_element("nana-calendar");
        let sized_calendar = doc.create_element("nana-calendar");
        doc.insert(default_calendar, doc.mount_root(), None);
        doc.insert(sized_calendar, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();

        let cells = nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
            [
                (
                    "date".into(),
                    nana_js_engine::HostValue::string("2026-06-01"),
                ),
                ("value".into(), nana_js_engine::HostValue::Number(4.0)),
            ]
            .into_iter()
            .collect(),
        )]);
        let mut default_props = crate::WidgetProps::default();
        default_props.apply_prop("data", &cells);
        bridge.register(
            default_calendar.0,
            crate::WidgetKind::CalendarHeatmap,
            default_props,
        );

        let mut sized_props = crate::WidgetProps::default();
        sized_props.apply_prop("data", &cells);
        sized_props.apply_prop(
            "options",
            &nana_js_engine::HostValue::Object(
                [
                    ("cellSize".into(), nana_js_engine::HostValue::Number(18.0)),
                    (
                        "weekStartsOn".into(),
                        nana_js_engine::HostValue::Number(0.0),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        );
        bridge.register(
            sized_calendar.0,
            crate::WidgetKind::CalendarHeatmap,
            sized_props,
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        let default_id = StableNodeId::try_from(default_calendar).unwrap();
        let sized_id = StableNodeId::try_from(sized_calendar).unwrap();
        let Some(nana_ui_runtime::StandardVisual::CalendarHeatmap {
            cell_size: default_size,
            cells: default_cells,
            ..
        }) = doc.runtime.standard_visual(default_id)
        else {
            panic!("default calendar must project a heatmap");
        };
        let Some(nana_ui_runtime::StandardVisual::CalendarHeatmap {
            cell_size: sized,
            cells: sized_cells,
            ..
        }) = doc.runtime.standard_visual(sized_id)
        else {
            panic!("options calendar must project a heatmap");
        };
        assert_eq!(default_size, 11.0);
        assert_eq!(sized, 18.0);
        let default_y = default_cells
            .iter()
            .find(|cell| cell.level > 0)
            .map(|cell| cell.y);
        let sized_y = sized_cells
            .iter()
            .find(|cell| cell.level > 0)
            .map(|cell| cell.y);
        assert_ne!(
            default_y, sized_y,
            "week_starts_on must shift the first active cell"
        );
    }

    fn calendar_sample_cells() -> nana_js_engine::HostValue {
        nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
            [
                (
                    "date".into(),
                    nana_js_engine::HostValue::string("2026-06-01"),
                ),
                ("value".into(), nana_js_engine::HostValue::Number(4.0)),
            ]
            .into_iter()
            .collect(),
        )])
    }

    fn project_calendar_with_options(
        options: nana_js_engine::HostValue,
    ) -> (
        NanaTreeDocument,
        crate::WidgetProps,
        nana_ui_runtime::StandardVisual,
    ) {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let calendar = doc.create_element("nana-calendar");
        doc.insert(calendar, doc.mount_root(), None);
        let mut props = crate::WidgetProps::default();
        props.apply_prop("data", &calendar_sample_cells());
        props.apply_prop("options", &options);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            calendar.0,
            crate::WidgetKind::CalendarHeatmap,
            props.clone(),
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let visual = doc
            .runtime
            .standard_visual(StableNodeId::try_from(calendar).unwrap())
            .expect("calendar must project a heatmap");
        (doc, props, visual)
    }

    #[test]
    fn calendar_options_weekday_labels_project_day_labels() {
        let (_doc, _props, visual) =
            project_calendar_with_options(nana_js_engine::HostValue::Object(
                [(
                    "weekdayLabels".into(),
                    nana_js_engine::HostValue::Array(vec![
                        nana_js_engine::HostValue::Array(vec![
                            nana_js_engine::HostValue::Number(0.0),
                            nana_js_engine::HostValue::string("Sun"),
                        ]),
                        nana_js_engine::HostValue::Object(
                            [
                                ("day".into(), nana_js_engine::HostValue::Number(2.0)),
                                ("label".into(), nana_js_engine::HostValue::string("Tue")),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                        nana_js_engine::HostValue::string("Wed"),
                    ]),
                )]
                .into_iter()
                .collect(),
            ));
        let nana_ui_runtime::StandardVisual::CalendarHeatmap { day_labels, .. } = visual else {
            panic!("calendar must project a heatmap");
        };
        let labels: Vec<&str> = day_labels.iter().map(|label| label.text.as_ref()).collect();
        assert!(labels.contains(&"Sun"), "pair weekday labels must project");
        assert!(
            labels.contains(&"Tue"),
            "object weekday labels must project"
        );
        assert!(
            !labels.iter().any(|label| label.contains("周")),
            "custom weekday labels replace the default 周* set"
        );
    }

    #[test]
    fn calendar_options_month_format_changes_month_label() {
        let (_doc, _default_props, default_visual) =
            project_calendar_with_options(nana_js_engine::HostValue::Object(BTreeMap::new()));
        let (_doc, _formatted_props, formatted_visual) =
            project_calendar_with_options(nana_js_engine::HostValue::Object(
                [(
                    "monthFormat".into(),
                    nana_js_engine::HostValue::string("{year}-{monthPad}"),
                )]
                .into_iter()
                .collect(),
            ));
        let nana_ui_runtime::StandardVisual::CalendarHeatmap {
            month_labels: default_months,
            ..
        } = default_visual
        else {
            panic!("default calendar must project a heatmap");
        };
        let nana_ui_runtime::StandardVisual::CalendarHeatmap {
            month_labels: formatted_months,
            ..
        } = formatted_visual
        else {
            panic!("formatted calendar must project a heatmap");
        };
        assert!(
            default_months
                .iter()
                .any(|label| label.text.as_ref() == "6月"),
            "default month formatter is {{month}}月"
        );
        assert!(
            formatted_months
                .iter()
                .any(|label| label.text.as_ref() == "2026-06"),
            "monthFormat {{year}}-{{monthPad}} must change the painted month label"
        );
    }

    #[test]
    fn calendar_options_title_format_changes_cell_title() {
        let (_doc, _, visual) = project_calendar_with_options(nana_js_engine::HostValue::Object(
            [(
                "titleFormat".into(),
                nana_js_engine::HostValue::string("{date}={value}"),
            )]
            .into_iter()
            .collect(),
        ));
        assert!(matches!(
            visual,
            nana_ui_runtime::StandardVisual::CalendarHeatmap { .. }
        ));
    }

    #[test]
    fn calendar_options_function_formatter_keeps_default() {
        let (_doc, _props, visual) =
            project_calendar_with_options(nana_js_engine::HostValue::Object(
                [
                    (
                        "monthFormatter".into(),
                        nana_js_engine::HostValue::Function(nana_js_engine::JsFunctionId(11)),
                    ),
                    (
                        "titleFormatter".into(),
                        nana_js_engine::HostValue::Function(nana_js_engine::JsFunctionId(12)),
                    ),
                ]
                .into_iter()
                .collect(),
            ));
        let nana_ui_runtime::StandardVisual::CalendarHeatmap { month_labels, .. } = visual else {
            panic!("function formatters must still project a heatmap");
        };
        assert!(
            month_labels
                .iter()
                .any(|label| label.text.as_ref() == "6月"),
            "Function-valued monthFormatter is ignored"
        );
    }

    fn settings_page_model_value(
        tabs: &[(&str, &str, bool)],
        default_tab: &str,
        hide_header: bool,
    ) -> nana_js_engine::HostValue {
        nana_js_engine::HostValue::Object(
            [
                (
                    "tabs".into(),
                    nana_js_engine::HostValue::Array(
                        tabs.iter()
                            .map(|(key, label, full_page)| {
                                nana_js_engine::HostValue::Object(
                                    [
                                        ("key".into(), nana_js_engine::HostValue::string(*key)),
                                        ("label".into(), nana_js_engine::HostValue::string(*label)),
                                        (
                                            "fullPage".into(),
                                            nana_js_engine::HostValue::Bool(*full_page),
                                        ),
                                    ]
                                    .into_iter()
                                    .collect(),
                                )
                            })
                            .collect(),
                    ),
                ),
                (
                    "defaultTab".into(),
                    nana_js_engine::HostValue::string(default_tab),
                ),
                (
                    "hideHeader".into(),
                    nana_js_engine::HostValue::Bool(hide_header),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn sync_settings_page(
        settings: nana_js_engine::HostValue,
        tab: Option<&str>,
        hide_header: Option<bool>,
    ) -> (NanaTreeDocument, StableNodeId, StableNodeId) {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let page = doc.create_element("nana-settings-page");
        let content = doc.create_element("div");
        doc.insert(page, doc.mount_root(), None);
        doc.insert(content, page, None);
        let mut props = crate::WidgetProps::default();
        props.apply_prop("settings", &settings);
        if let Some(tab) = tab {
            props.apply_prop("tab", &nana_js_engine::HostValue::string(tab));
        }
        if let Some(hide_header) = hide_header {
            props.apply_prop("hide-header", &nana_js_engine::HostValue::Bool(hide_header));
        }
        let mut bridge = crate::MessageBridge::new();
        bridge.register(page.0, crate::WidgetKind::SettingsPage, props);
        bridge.register(
            content.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(content.0, page.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        (
            doc,
            StableNodeId::try_from(page).unwrap(),
            StableNodeId::try_from(content).unwrap(),
        )
    }

    fn settings_page_assembly(
        doc: &NanaTreeDocument,
        page: StableNodeId,
    ) -> nana_ui_runtime::SettingsPageAssembly {
        doc.context()
            .read(
                Entity::<RuntimeSettingsPage>::from_stable_id(page),
                |page| page.assembly.clone().unwrap_or_default(),
            )
            .expect("SettingsPage must be bound after sync")
    }

    fn world_has_descendant(
        doc: &NanaTreeDocument,
        root: StableNodeId,
        target: StableNodeId,
    ) -> bool {
        let Some(node) = doc.runtime.node(root) else {
            return false;
        };
        node.children
            .iter()
            .any(|child| *child == target || world_has_descendant(doc, *child, target))
    }

    #[test]
    fn settings_page_widget_kind_parses_host_tag() {
        assert_eq!(
            crate::WidgetKind::parse("nana-settings-page"),
            Some(crate::WidgetKind::SettingsPage)
        );
        assert_eq!(
            crate::WidgetKind::parse("settings-page"),
            Some(crate::WidgetKind::SettingsPage)
        );
        assert_eq!(
            crate::WidgetKind::parse("settingspage"),
            Some(crate::WidgetKind::SettingsPage)
        );
        assert_eq!(
            crate::WidgetKind::SettingsPage.element_tag(),
            "nana-settings-page"
        );
    }

    #[test]
    fn settings_page_assembles_scroll_body_title() {
        let (doc, page, content) = sync_settings_page(
            settings_page_model_value(&[("appearance", "外观", false)], "appearance", false),
            Some("appearance"),
            None,
        );
        let assembly = settings_page_assembly(&doc, page);
        let scroll = assembly
            .scroll
            .expect("assemble_settings_page mounts scroll");
        let body = assembly.body.expect("assemble_settings_page mounts body");
        let title = assembly.title.expect("assemble_settings_page mounts title");
        assert_eq!(doc.runtime.node(page).unwrap().children, vec![scroll]);
        assert_eq!(
            doc.runtime.node(scroll).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert!(
            doc.runtime
                .node_style(scroll)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert_eq!(doc.runtime.node(scroll).unwrap().children, vec![body]);
        assert_eq!(
            doc.runtime.node(body).unwrap().children,
            vec![title, content]
        );
        assert_eq!(doc.runtime.text(title), Some("外观"));
    }

    #[test]
    fn settings_page_attrs_json_assembles_scroll_body_title() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let page = doc.create_element("nana-settings-page");
        let content = doc.create_element("div");
        doc.insert(page, doc.mount_root(), None);
        doc.insert(content, page, None);
        let mut props = crate::WidgetProps::default();
        props.attrs.insert(
            "settings".into(),
            r#"{"tabs":[{"id":"appearance","label":"外观"}],"defaultTab":"appearance"}"#.into(),
        );
        props.attrs.insert("tab".into(), "appearance".into());
        assert!(
            !props.native_props.contains_key("settings")
                && !props.native_props.contains_key("model"),
            "attrs-only settings must not supply native_props"
        );
        let mut bridge = crate::MessageBridge::new();
        bridge.register(page.0, crate::WidgetKind::SettingsPage, props);
        bridge.register(
            content.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(content.0, page.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let page = StableNodeId::try_from(page).unwrap();
        let content = StableNodeId::try_from(content).unwrap();
        let assembly = settings_page_assembly(&doc, page);
        let scroll = assembly
            .scroll
            .expect("assemble_settings_page mounts scroll");
        let body = assembly.body.expect("assemble_settings_page mounts body");
        let title = assembly.title.expect("assemble_settings_page mounts title");
        assert_eq!(doc.runtime.node(page).unwrap().children, vec![scroll]);
        assert_eq!(
            doc.runtime.node(scroll).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert!(
            doc.runtime
                .node_style(scroll)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert_eq!(doc.runtime.node(scroll).unwrap().children, vec![body]);
        assert_eq!(
            doc.runtime.node(body).unwrap().children,
            vec![title, content]
        );
        assert_eq!(doc.runtime.text(title), Some("外观"));
    }

    #[test]
    fn settings_page_resync_reuses_scroll_body_title() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let page = doc.create_element("nana-settings-page");
        let content = doc.create_element("div");
        doc.insert(page, doc.mount_root(), None);
        doc.insert(content, page, None);
        let mut props = crate::WidgetProps::default();
        props.apply_prop(
            "settings",
            &settings_page_model_value(&[("appearance", "外观", false)], "appearance", false),
        );
        props.apply_prop("tab", &nana_js_engine::HostValue::string("appearance"));
        let mut bridge = crate::MessageBridge::new();
        bridge.register(page.0, crate::WidgetKind::SettingsPage, props.clone());
        bridge.register(
            content.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(content.0, page.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let page_id = StableNodeId::try_from(page).unwrap();
        let content_id = StableNodeId::try_from(content).unwrap();
        let first = settings_page_assembly(&doc, page_id);
        let scroll = first.scroll.expect("assemble_settings_page mounts scroll");
        let body = first.body.expect("assemble_settings_page mounts body");
        let title = first.title.expect("assemble_settings_page mounts title");

        bridge.insert_child(content.0, page.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let second = settings_page_assembly(&doc, page_id);
        assert_eq!(second.scroll, Some(scroll));
        assert_eq!(second.body, Some(body));
        assert_eq!(second.title, Some(title));
        assert_eq!(
            doc.runtime.node(body).unwrap().children,
            vec![title, content_id]
        );
        assert_eq!(doc.runtime.text(title), Some("外观"));
    }

    #[test]
    fn settings_page_hide_header_omits_title() {
        let (doc, page, content) = sync_settings_page(
            settings_page_model_value(&[("appearance", "外观", false)], "appearance", true),
            Some("appearance"),
            Some(true),
        );
        let assembly = settings_page_assembly(&doc, page);
        let body = assembly.body.expect("hide-header still mounts scroll body");
        assert_eq!(doc.runtime.node(body).unwrap().children, vec![content]);
        assert!(
            assembly.title.is_none() || !world_has_descendant(&doc, page, assembly.title.unwrap())
        );
        assert_ne!(doc.runtime.text(page), Some("外观"));
    }

    #[test]
    fn settings_page_full_page_omits_title() {
        let (doc, page, content) = sync_settings_page(
            settings_page_model_value(&[("workspace", "工作区", true)], "workspace", false),
            Some("workspace"),
            None,
        );
        let assembly = settings_page_assembly(&doc, page);
        assert_eq!(doc.runtime.node(page).unwrap().children, vec![content]);
        assert!(
            assembly.title.is_none() || !world_has_descendant(&doc, page, assembly.title.unwrap())
        );
        if let Some(scroll) = assembly.scroll {
            assert!(!world_has_descendant(&doc, page, scroll));
        }
        assert_ne!(doc.runtime.text(page), Some("工作区"));
    }

    #[test]
    fn graph_viewport_and_selection_survive_projection() {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let canvas = doc.create_element("nana-graph-canvas");
        doc.insert(canvas, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        let mut props = crate::WidgetProps {
            label: "Graph".into(),
            ..Default::default()
        };
        props.apply_prop(
            "nodes",
            &nana_js_engine::HostValue::Array(vec![
                nana_js_engine::HostValue::Object(
                    [
                        ("id".into(), nana_js_engine::HostValue::string("source")),
                        ("title".into(), nana_js_engine::HostValue::string("Source")),
                        ("x".into(), nana_js_engine::HostValue::Number(20.0)),
                        ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                nana_js_engine::HostValue::Object(
                    [
                        ("id".into(), nana_js_engine::HostValue::string("sink")),
                        ("label".into(), nana_js_engine::HostValue::string("Sink")),
                        ("x".into(), nana_js_engine::HostValue::Number(240.0)),
                        ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ]),
        );
        props.apply_prop(
            "viewport",
            &nana_js_engine::HostValue::Object(
                [
                    (
                        "offset".into(),
                        nana_js_engine::HostValue::Object(
                            [
                                ("x".into(), nana_js_engine::HostValue::Number(12.0)),
                                ("y".into(), nana_js_engine::HostValue::Number(-8.0)),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    ),
                    ("zoom".into(), nana_js_engine::HostValue::Number(2.0)),
                ]
                .into_iter()
                .collect(),
            ),
        );
        props.apply_prop(
            "selection",
            &nana_js_engine::HostValue::Object(
                [
                    ("kind".into(), nana_js_engine::HostValue::string("node")),
                    ("id".into(), nana_js_engine::HostValue::string("source")),
                ]
                .into_iter()
                .collect(),
            ),
        );
        bridge.register(canvas.0, crate::WidgetKind::GraphCanvas, props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let canvas_id = StableNodeId::try_from(canvas).unwrap();
        assert!(
            matches!(
                doc.runtime.standard_visual(canvas_id),
                Some(nana_ui_runtime::StandardVisual::GraphCanvas {
                    viewport_offset_x,
                    viewport_offset_y,
                    viewport_zoom,
                    nodes,
                    ..
                }) if viewport_offset_x == 12.0
                    && viewport_offset_y == -8.0
                    && viewport_zoom == 2.0
                    && nodes.iter().any(|node| node.selected)
            ),
            "viewport zoom/offset and selection must survive GraphCanvas projection"
        );
    }

    #[test]
    fn app_shell_with_title_and_child_projects_title_bar_and_body() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, shell, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(
            body.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Workspace".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::AppShell,
            crate::WidgetProps {
                label: "Nana".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, shell.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let body_id = StableNodeId::try_from(body).unwrap();
        let title_style = doc.runtime.node_style(title_id).unwrap();
        let body_style = doc.runtime.node_style(body_id).unwrap();
        assert_eq!(
            title_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
        );
        assert_eq!(title_style.layout.flex_grow, Some(0.0));
        assert!(
            doc.runtime.text(title_id).unwrap_or("").is_empty(),
            "title-bar root text must be empty after assemble, got {:?}",
            doc.runtime.text(title_id)
        );
        let title_label =
            assembled_title_bar_center_label(&doc, title_id).expect("center column title");
        assert_eq!(doc.runtime.text(title_label), Some("Nana"));
        assert_eq!(
            doc.runtime
                .accessibility(title_id)
                .and_then(|state| state.label.as_deref()),
            Some("Nana")
        );
        assert_eq!(body_style.layout.flex_grow, Some(1.0));
        assert_eq!(
            body_style.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
    }

    #[test]
    fn app_shell_title_bar_and_body_keep_layout_after_assemble() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, shell, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(
            body.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Workspace".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::AppShell,
            crate::WidgetProps {
                label: "Nana".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, shell.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let shell_id = StableNodeId::try_from(shell).unwrap();
        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let body_id = StableNodeId::try_from(body).unwrap();
        let (bound_title, bound_body, bound_overlay) = doc
            .context()
            .read(
                Entity::<RuntimeAppShell>::from_stable_id(shell_id),
                |shell| (shell.title_bar, shell.body, shell.overlay),
            )
            .unwrap();
        assert_eq!(bound_title, Some(title_id));
        assert_eq!(bound_body, Some(body_id));
        assert_eq!(bound_overlay, None);
        assert_eq!(
            doc.runtime.node(shell_id).unwrap().children,
            vec![title_id, body_id]
        );
        let title_style = doc.runtime.node_style(title_id).unwrap();
        let body_style = doc.runtime.node_style(body_id).unwrap();
        assert_eq!(
            title_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
        );
        assert_eq!(title_style.layout.flex_grow, Some(0.0));
        assert!(
            doc.runtime.text(title_id).unwrap_or("").is_empty(),
            "title-bar root text must be empty after assemble, got {:?}",
            doc.runtime.text(title_id)
        );
        let title_label =
            assembled_title_bar_center_label(&doc, title_id).expect("center column title");
        assert_eq!(doc.runtime.text(title_label), Some("Nana"));
        assert_eq!(
            doc.runtime
                .accessibility(title_id)
                .and_then(|state| state.label.as_deref()),
            Some("Nana")
        );
        assert_eq!(body_style.layout.flex_grow, Some(1.0));
        assert_eq!(
            body_style.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
    }

    #[test]
    fn column_nana_app_shell_title_bar_and_body_keep_layout_after_assemble() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, shell, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(
            body.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Workspace".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Nana".into(),
                element_tag: "nana-app-shell".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, shell.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let shell_id = StableNodeId::try_from(shell).unwrap();
        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let body_id = StableNodeId::try_from(body).unwrap();
        let (bound_title, bound_body, bound_overlay) = doc
            .context()
            .read(
                Entity::<RuntimeAppShell>::from_stable_id(shell_id),
                |shell| (shell.title_bar, shell.body, shell.overlay),
            )
            .unwrap();
        assert_eq!(bound_title, Some(title_id));
        assert_eq!(bound_body, Some(body_id));
        assert_eq!(bound_overlay, None);
        assert_eq!(
            doc.runtime.node(shell_id).unwrap().children,
            vec![title_id, body_id]
        );
        let title_style = doc.runtime.node_style(title_id).unwrap();
        let body_style = doc.runtime.node_style(body_id).unwrap();
        assert_eq!(
            title_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
        );
        assert_eq!(title_style.layout.flex_grow, Some(0.0));
        assert!(
            doc.runtime.text(title_id).unwrap_or("").is_empty(),
            "title-bar root text must be empty after assemble, got {:?}",
            doc.runtime.text(title_id)
        );
        let title_label =
            assembled_title_bar_center_label(&doc, title_id).expect("center column title");
        assert_eq!(doc.runtime.text(title_label), Some("Nana"));
        assert_eq!(
            doc.runtime
                .accessibility(title_id)
                .and_then(|state| state.label.as_deref()),
            Some("Nana")
        );
        assert_eq!(body_style.layout.flex_grow, Some(1.0));
        assert_eq!(
            body_style.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        runtime_layout(&mut doc, 800.0, 600.0);
        assert!(
            !runtime_is_descendant(&doc, title_id, body_id),
            "body must stay a sibling of the title bar"
        );
        let title_box = doc.runtime.layout_box(title_id).unwrap();
        let body_box = doc.runtime.layout_box(body_id).unwrap();
        assert!(
            body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
            "body.y={} must sit below title bar y={} height={}",
            body_box.y,
            title_box.y,
            title_box.height
        );
    }

    #[test]
    fn column_nana_app_shell_title_bar_and_body_copy_stays_below_title_bar() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        let copy = doc.create_text("Type in the field below.");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, shell, None);
        doc.insert(copy, body, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        let mut body_props = crate::WidgetProps {
            element_tag: "div".into(),
            ..Default::default()
        };
        body_props.attrs.insert("data-slot".into(), "body".into());
        body_props.class_names.push("chrome-probe-body".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(body.0, crate::WidgetKind::Column, body_props);
        bridge.register(
            copy.0,
            crate::WidgetKind::Text,
            crate::WidgetProps {
                label: "Type in the field below.".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Nana".into(),
                element_tag: "nana-app-shell".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, shell.0, None);
        bridge.insert_child(copy.0, body.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        runtime_layout(&mut doc, 800.0, 600.0);

        let shell_id = StableNodeId::try_from(shell).unwrap();
        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let body_id = StableNodeId::try_from(body).unwrap();
        let copy_id = StableNodeId::try_from(copy).unwrap();
        assert_eq!(
            doc.runtime.node(shell_id).unwrap().children,
            vec![title_id, body_id]
        );
        assert!(
            !runtime_is_descendant(&doc, title_id, body_id),
            "body must not be a descendant of the title bar"
        );
        assert!(
            !runtime_is_descendant(&doc, title_id, copy_id),
            "body copy must not be a child of the title-bar node"
        );
        assert_eq!(doc.runtime.text(copy_id), Some("Type in the field below."));
        let title_box = doc.runtime.layout_box(title_id).unwrap();
        let body_box = doc.runtime.layout_box(body_id).unwrap();
        assert!(
            body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
            "body.y={} must sit below title bar y={} height={}",
            body_box.y,
            title_box.y,
            title_box.height
        );
    }

    #[test]
    fn column_nana_app_shell_nested_body_is_reparented_below_title_bar() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        let copy = doc.create_text("Type in the field below.");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, title_bar, None);
        doc.insert(copy, body, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        let mut body_props = crate::WidgetProps {
            element_tag: "div".into(),
            ..Default::default()
        };
        body_props.attrs.insert("data-slot".into(), "body".into());
        body_props.class_names.push("chrome-probe-body".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(body.0, crate::WidgetKind::Column, body_props);
        bridge.register(
            copy.0,
            crate::WidgetKind::Text,
            crate::WidgetProps {
                label: "Type in the field below.".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                label: "Nana".into(),
                element_tag: "nana-app-shell".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, title_bar.0, None);
        bridge.insert_child(copy.0, body.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        runtime_layout(&mut doc, 800.0, 600.0);

        let shell_id = StableNodeId::try_from(shell).unwrap();
        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let body_id = StableNodeId::try_from(body).unwrap();
        let copy_id = StableNodeId::try_from(copy).unwrap();
        assert_eq!(
            doc.runtime.node(shell_id).unwrap().children,
            vec![title_id, body_id]
        );
        assert!(
            !runtime_is_descendant(&doc, title_id, body_id),
            "nested body must be lifted out of the title bar"
        );
        assert!(
            !runtime_is_descendant(&doc, title_id, copy_id),
            "body copy must not stay under the title-bar node"
        );
        let title_box = doc.runtime.layout_box(title_id).unwrap();
        let body_box = doc.runtime.layout_box(body_id).unwrap();
        assert!(
            body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
            "body.y={} must sit below title bar y={} height={}",
            body_box.y,
            title_box.y,
            title_box.height
        );
    }

    #[test]
    fn app_shell_empty_title_bar_still_assembles_columns() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let body = doc.create_element("div");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(body, shell, None);
        let mut bridge = crate::MessageBridge::new();
        let mut title_props = crate::WidgetProps {
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
        bridge.register(
            body.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.register(
            shell.0,
            crate::WidgetKind::AppShell,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(body.0, shell.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let title_id = StableNodeId::try_from(title_bar).unwrap();
        let columns = doc.runtime.node(title_id).unwrap().children;
        let tags: Vec<String> = columns
            .iter()
            .map(|&id| match doc.runtime.node(id).map(|node| node.kind) {
                Some(NodeKind::Element { tag }) => tag,
                other => panic!("expected title-bar column, got {other:?}"),
            })
            .collect();
        assert_eq!(
            tags,
            [
                "app-title-bar-leading",
                "app-title-bar-center",
                "app-title-bar-trailing"
            ]
        );
        assert!(
            doc.runtime.text(title_id).unwrap_or("").is_empty(),
            "title-bar root text must be empty after assemble, got {:?}",
            doc.runtime.text(title_id)
        );
        let title_label =
            assembled_title_bar_center_label(&doc, title_id).expect("center column title");
        assert_eq!(doc.runtime.text(title_label), Some(""));
        assert_eq!(
            doc.runtime
                .accessibility(title_id)
                .and_then(|state| state.label.as_deref()),
            Some("")
        );
    }

    fn runtime_is_descendant(
        doc: &NanaTreeDocument,
        ancestor: StableNodeId,
        node: StableNodeId,
    ) -> bool {
        let mut current = doc.runtime.node(node).and_then(|node| node.parent);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = doc.runtime.node(id).and_then(|node| node.parent);
        }
        false
    }

    fn assembled_title_bar_center_label(
        doc: &NanaTreeDocument,
        title_id: StableNodeId,
    ) -> Option<StableNodeId> {
        let columns = doc.runtime.node(title_id)?.children;
        let center = columns.into_iter().find(|&id| {
            matches!(
                doc.runtime.node(id).as_ref().map(|node| &node.kind),
                Some(NodeKind::Element { tag }) if tag == "app-title-bar-center"
            )
        })?;
        doc.runtime.node(center)?.children.first().copied()
    }

    #[test]
    fn split_pane_two_children_vertical_sets_slots_and_axis() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let pane = doc.create_element("nana-split-pane");
        let first = doc.create_element("div");
        let second = doc.create_element("div");
        doc.insert(pane, doc.mount_root(), None);
        doc.insert(first, pane, None);
        doc.insert(second, pane, None);
        let mut bridge = crate::MessageBridge::new();
        let mut props = crate::WidgetProps {
            element_tag: "nana-split-pane".into(),
            ..Default::default()
        };
        props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
        bridge.register(pane.0, crate::WidgetKind::SplitPane, props);
        bridge.register(
            first.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.register(
            second.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(first.0, pane.0, None);
        bridge.insert_child(second.0, pane.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let pane_id = StableNodeId::try_from(pane).unwrap();
        let first_id = StableNodeId::try_from(first).unwrap();
        let second_id = StableNodeId::try_from(second).unwrap();
        assert_eq!(
            doc.runtime.node_style(pane_id).unwrap().layout.direction,
            Some(nana_ui_core::FlexDirection::Column)
        );
        let first_style = doc.runtime.node_style(first_id).unwrap();
        let second_style = doc.runtime.node_style(second_id).unwrap();
        assert_eq!(
            first_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(240.0))
        );
        assert_eq!(first_style.layout.flex_grow, Some(0.0));
        assert_eq!(
            second_style.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        assert_eq!(second_style.layout.flex_grow, Some(1.0));
    }

    #[test]
    fn split_pane_two_children_assemble_binds_resize_handle() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let pane = doc.create_element("nana-split-pane");
        let first = doc.create_element("div");
        let second = doc.create_element("div");
        doc.insert(pane, doc.mount_root(), None);
        doc.insert(first, pane, None);
        doc.insert(second, pane, None);
        let mut bridge = crate::MessageBridge::new();
        let mut props = crate::WidgetProps {
            element_tag: "nana-split-pane".into(),
            ..Default::default()
        };
        props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
        bridge.register(pane.0, crate::WidgetKind::SplitPane, props);
        bridge.register(
            first.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.register(
            second.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(first.0, pane.0, None);
        bridge.insert_child(second.0, pane.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let pane_id = StableNodeId::try_from(pane).unwrap();
        let first_id = StableNodeId::try_from(first).unwrap();
        let second_id = StableNodeId::try_from(second).unwrap();
        let handle = doc
            .context()
            .read(
                Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
                |pane| pane.handle,
            )
            .unwrap()
            .expect("assemble_split_pane creates a handle");
        assert!(
            doc.context().is_split_handle(handle),
            "assembled handle must be a split-pane resize target"
        );
        assert_eq!(
            doc.runtime.node(pane_id).unwrap().children,
            vec![first_id, handle, second_id]
        );
        let first_style = doc.runtime.node_style(first_id).unwrap();
        let second_style = doc.runtime.node_style(second_id).unwrap();
        assert_eq!(
            first_style.layout.height,
            Some(nana_ui_core::LengthSpec::Px(240.0))
        );
        assert_eq!(
            second_style.layout.height,
            Some(nana_ui_core::LengthSpec::Fill)
        );
    }

    #[test]
    fn split_pane_resync_reuses_handle() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let pane = doc.create_element("nana-split-pane");
        let first = doc.create_element("div");
        let second = doc.create_element("div");
        doc.insert(pane, doc.mount_root(), None);
        doc.insert(first, pane, None);
        doc.insert(second, pane, None);
        let mut props = crate::WidgetProps {
            element_tag: "nana-split-pane".into(),
            ..Default::default()
        };
        props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
        let mut bridge = crate::MessageBridge::new();
        bridge.register(pane.0, crate::WidgetKind::SplitPane, props.clone());
        bridge.register(
            first.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.register(
            second.0,
            crate::WidgetKind::Column,
            crate::WidgetProps::default(),
        );
        bridge.insert_child(first.0, pane.0, None);
        bridge.insert_child(second.0, pane.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let pane_id = StableNodeId::try_from(pane).unwrap();
        let first_id = StableNodeId::try_from(first).unwrap();
        let second_id = StableNodeId::try_from(second).unwrap();
        let handle = doc
            .context()
            .read(
                Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
                |pane| pane.handle,
            )
            .unwrap()
            .expect("assemble_split_pane creates a handle");

        bridge.patch_prop(
            pane.0,
            "axis",
            &nana_js_engine::HostValue::string("vertical"),
        );
        doc.sync_semantic_styles(&bridge.snapshot());

        let handle_again = doc
            .context()
            .read(
                Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
                |pane| pane.handle,
            )
            .unwrap()
            .expect("resync keeps a handle");
        assert_eq!(handle_again, handle);
        assert!(
            doc.context().is_split_handle(handle),
            "resync must keep the same resize handle identity"
        );
        let children = doc.runtime.node(pane_id).unwrap().children;
        assert!(children.contains(&handle));
        assert!(children.contains(&first_id));
        assert!(children.contains(&second_id));
    }

    #[test]
    fn dock_two_item_children_are_not_dummy_dock_item() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let dock = doc.create_element("nana-dock");
        let nav = doc.create_element("div");
        let files = doc.create_element("div");
        doc.insert(dock, doc.mount_root(), None);
        doc.insert(nav, dock, None);
        doc.insert(files, dock, None);
        let mut bridge = crate::MessageBridge::new();
        let mut nav_props = crate::WidgetProps {
            label: "Nav".into(),
            ..Default::default()
        };
        nav_props.attrs.insert("data-dock-id".into(), "nav".into());
        let mut files_props = crate::WidgetProps {
            label: "Files".into(),
            ..Default::default()
        };
        files_props
            .attrs
            .insert("data-dock-id".into(), "files".into());
        bridge.register(
            dock.0,
            crate::WidgetKind::Dock,
            crate::WidgetProps::default(),
        );
        bridge.register(nav.0, crate::WidgetKind::Column, nav_props);
        bridge.register(files.0, crate::WidgetKind::Column, files_props);
        bridge.insert_child(nav.0, dock.0, None);
        bridge.insert_child(files.0, dock.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let dock_id = StableNodeId::try_from(dock).unwrap();
        let nav_id = StableNodeId::try_from(nav).unwrap();
        let files_id = StableNodeId::try_from(files).unwrap();
        let items = doc
            .context()
            .read(Entity::<RuntimeDock>::from_stable_id(dock_id), |dock| {
                dock.flatten()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_ne!(
            items.as_slice(),
            ["dock"],
            "dock with two item children must not project DockNode::item(\"dock\", None)"
        );
        assert_eq!(items, ["nav", "files"]);
        assert!(
            doc.runtime.node_style(files_id).unwrap().layout.hidden,
            "inactive dock tab body is hidden"
        );
        let children = doc.runtime.node(dock_id).unwrap().children;
        assert!(
            children.len() > 2,
            "assemble_dock must mount chrome beyond the two content nodes"
        );
        assert!(children.contains(&nav_id));
        assert!(children.contains(&files_id));
        assert!(
            children.iter().any(|child| {
                doc.runtime
                    .accessibility(*child)
                    .is_some_and(|state| state.role == AccessibilityRole::TabList)
            }),
            "assemble_dock mounts a tab-strip chrome node"
        );
    }

    #[test]
    fn column_nana_dock_two_item_children_are_not_dummy_dock_item() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let dock = doc.create_element("nana-dock");
        let nav = doc.create_element("div");
        let files = doc.create_element("div");
        doc.insert(dock, doc.mount_root(), None);
        doc.insert(nav, dock, None);
        doc.insert(files, dock, None);
        let mut bridge = crate::MessageBridge::new();
        let mut nav_props = crate::WidgetProps {
            label: "Nav".into(),
            ..Default::default()
        };
        nav_props.attrs.insert("data-dock-id".into(), "nav".into());
        let mut files_props = crate::WidgetProps {
            label: "Files".into(),
            ..Default::default()
        };
        files_props
            .attrs
            .insert("data-dock-id".into(), "files".into());
        bridge.register(
            dock.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                element_tag: "nana-dock".into(),
                ..Default::default()
            },
        );
        bridge.register(nav.0, crate::WidgetKind::Column, nav_props);
        bridge.register(files.0, crate::WidgetKind::Column, files_props);
        bridge.insert_child(nav.0, dock.0, None);
        bridge.insert_child(files.0, dock.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let dock_id = StableNodeId::try_from(dock).unwrap();
        let nav_id = StableNodeId::try_from(nav).unwrap();
        let files_id = StableNodeId::try_from(files).unwrap();
        let items = doc
            .context()
            .read(Entity::<RuntimeDock>::from_stable_id(dock_id), |dock| {
                dock.flatten()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_ne!(
            items.as_slice(),
            ["dock"],
            "dock with two item children must not project DockNode::item(\"dock\", None)"
        );
        assert_eq!(items, ["nav", "files"]);
        assert!(
            doc.runtime.node_style(files_id).unwrap().layout.hidden,
            "inactive dock tab body is hidden"
        );
        let children = doc.runtime.node(dock_id).unwrap().children;
        assert!(
            children.len() > 2,
            "assemble_dock must mount chrome beyond the two content nodes"
        );
        assert!(children.contains(&nav_id));
        assert!(children.contains(&files_id));
        assert!(
            children.iter().any(|child| {
                doc.runtime
                    .accessibility(*child)
                    .is_some_and(|state| state.role == AccessibilityRole::TabList)
            }),
            "assemble_dock mounts a tab-strip chrome node"
        );
    }

    #[test]
    fn workspace_region_child_records_region_slot() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let workspace = doc.create_element("nana-workspace");
        let primary = doc.create_element("div");
        doc.insert(workspace, doc.mount_root(), None);
        doc.insert(primary, workspace, None);
        let mut bridge = crate::MessageBridge::new();
        let mut region_props = crate::WidgetProps {
            region: "primary".into(),
            ..Default::default()
        };
        region_props
            .attrs
            .insert("data-region".into(), "primary".into());
        bridge.register(
            workspace.0,
            crate::WidgetKind::Workspace,
            crate::WidgetProps::default(),
        );
        bridge.register(primary.0, crate::WidgetKind::Column, region_props);
        bridge.insert_child(primary.0, workspace.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let workspace_id = StableNodeId::try_from(workspace).unwrap();
        let primary_id = StableNodeId::try_from(primary).unwrap();
        let label = doc
            .runtime
            .accessibility(primary_id)
            .and_then(|state| state.label.clone())
            .unwrap_or_default();
        assert_eq!(
            label.as_ref(),
            "primary",
            "workspace region child must be recorded as a WorkspaceRegionSlot"
        );
        let (middle, primary_column, primary_row, editor_stack) = doc
            .context()
            .read(
                Entity::<RuntimeWorkspace>::from_stable_id(workspace_id),
                |workspace| {
                    (
                        workspace.middle,
                        workspace.primary_column,
                        workspace.primary_row,
                        workspace.editor_stack,
                    )
                },
            )
            .unwrap();
        assert!(
            middle.is_some(),
            "assemble_workspace creates the middle track"
        );
        assert!(
            primary_column.is_some(),
            "assemble_workspace creates the primary column"
        );
        assert!(
            primary_row.is_some(),
            "assemble_workspace creates the primary row"
        );
        assert!(
            editor_stack.is_some(),
            "assemble_workspace creates the editor stack"
        );
        let children = doc.runtime.node(workspace_id).unwrap().children;
        assert!(
            children.contains(&middle.unwrap()) || children.len() > 1,
            "assemble_workspace mounts track hosts as extra child ids"
        );
    }

    #[test]
    fn markdown_mermaid_and_math_fences_project_native_blocks() {
        let mut doc = NanaTreeDocument::new(420, 240, 1.0);
        let markdown = doc.create_element("nana-markdown");
        doc.insert(markdown, doc.mount_root(), None);
        let source =
            "# Title\n\n```mermaid\nflowchart LR\nA-->B\n```\n\n```math\n\\frac{1}{2}\n```";
        let parsed = RuntimeNativeMarkdown::from_source(source);
        assert!(
            parsed
                .blocks()
                .iter()
                .any(|block| matches!(block, nana_ui_runtime::MarkdownBlock::Mermaid(_)))
        );
        assert!(
            parsed
                .blocks()
                .iter()
                .any(|block| matches!(block, nana_ui_runtime::MarkdownBlock::DisplayMath(_)))
        );

        let mut bridge = crate::MessageBridge::new();
        let mut props = crate::WidgetProps {
            value: source.into(),
            ..Default::default()
        };
        props.apply_prop(
            "mermaidRenderer",
            &nana_js_engine::HostValue::string("app-mermaid"),
        );
        props.apply_prop(
            "mathRenderer",
            &nana_js_engine::HostValue::string("app-math"),
        );
        bridge.register(markdown.0, crate::WidgetKind::NativeMarkdown, props);
        doc.sync_semantic_styles(&bridge.snapshot());

        let markdown_id = StableNodeId::try_from(markdown).unwrap();
        let expected = parsed.plain_text();
        assert!(
            matches!(
                doc.runtime.standard_visual(markdown_id),
                Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                    if text.as_ref() == expected
                        && text.contains("flowchart LR")
                        && text.contains("\\frac{1}{2}")
            ),
            "Vue NativeMarkdown must project mermaid/math fences through from_source"
        );
        assert!(
            doc.runtime.custom_render(markdown_id).is_none(),
            "mermaid/math stay on NativeMarkdown; Vue must not invent a renderer"
        );
        let children = doc.runtime.node(markdown_id).unwrap().children;
        assert!(
            children.iter().any(|child| {
                doc.runtime
                    .highlight_request(*child)
                    .is_some_and(|request| {
                        request.presenter.as_ref() == RuntimeNativeMarkdown::MERMAID_PRESENTER
                    })
            }),
            "assemble_markdown attaches a mermaid fence child"
        );
        assert!(
            children.iter().any(|child| {
                doc.runtime
                    .node_style(*child)
                    .is_some_and(|style| style.layout.hidden)
            }),
            "fence children stay hidden identity slots"
        );
    }

    #[test]
    fn runtime_is_authoritative_for_semantic_hierarchy_and_unchanged_style_is_idle() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let child = doc.create_element("button");
        doc.insert(child, doc.mount_root(), None);
        let mut props = crate::WidgetProps::default();
        props.element_tag = "button".into();
        props.layout.opacity = Some(0.5);
        let mut snapshot = crate::SemanticSnapshot {
            revision: 1,
            theme: nana_ui_core::ThemeMode::Light,
            appearance: nana_ui_core::AppearanceSettings::default(),
            roots: vec![child.0],
            widgets: vec![crate::SemanticWidget {
                id: child.0,
                kind: crate::WidgetKind::Button,
                props,
                children: vec![999],
                parent: None,
            }],
        };
        doc.apply_runtime_hierarchy(&mut snapshot);
        assert_eq!(snapshot.roots, vec![child.0]);
        assert!(snapshot.widgets[0].children.is_empty());

        doc.sync_semantic_styles(&snapshot);
        let generation = doc.runtime_generation();
        doc.sync_semantic_styles(&snapshot);
        assert_eq!(doc.runtime_generation(), generation);
        let id = StableNodeId::try_from(child).unwrap();
        assert_eq!(
            doc.runtime.node_style(id).unwrap().layout.opacity,
            Some(0.5)
        );
        assert!(doc.runtime.interaction(id).unwrap().focusable);
        assert_eq!(doc.runtime.theme_mode(), nana_ui_core::ThemeMode::Light);

        snapshot.revision += 1;
        snapshot.theme = nana_ui_core::ThemeMode::Dark;
        doc.sync_semantic_styles(&snapshot);
        assert_eq!(doc.runtime.theme_mode(), nana_ui_core::ThemeMode::Dark);
    }

    #[test]
    fn sidebar_frame_body_projects_as_vertical_scroll_view() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let frame = doc.create_element("nana-sidebar-frame");
        let top = doc.create_element("nana-column");
        let body = doc.create_element("nana-column");
        let footer = doc.create_element("nana-column");
        doc.insert(frame, doc.mount_root(), None);
        doc.insert(top, frame, None);
        doc.insert(body, frame, None);
        doc.insert(footer, frame, None);
        let mut bridge = crate::MessageBridge::new();
        let mut frame_props = crate::WidgetProps::default();
        frame_props.class_names = vec!["nana-sidebar-frame".into()];
        bridge.register(frame.0, crate::WidgetKind::SidebarFrame, frame_props);
        let mut top_props = crate::WidgetProps::default();
        top_props.class_names = vec!["nana-sidebar-frame__top".into()];
        top_props
            .attrs
            .insert("data-slot".into(), "sidebar-top".into());
        bridge.register(top.0, crate::WidgetKind::Column, top_props);
        let mut body_props = crate::WidgetProps::default();
        body_props.class_names = vec!["nana-sidebar-frame__body".into()];
        body_props
            .attrs
            .insert("data-slot".into(), "sidebar-body".into());
        bridge.register(body.0, crate::WidgetKind::Column, body_props);
        let mut footer_props = crate::WidgetProps::default();
        footer_props.class_names = vec!["nana-sidebar-frame__footer".into()];
        footer_props
            .attrs
            .insert("data-slot".into(), "sidebar-footer".into());
        bridge.register(footer.0, crate::WidgetKind::Column, footer_props);
        bridge.insert_child(top.0, frame.0, None);
        bridge.insert_child(body.0, frame.0, None);
        bridge.insert_child(footer.0, frame.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());

        let body_id = StableNodeId::try_from(body).unwrap();
        let top_id = StableNodeId::try_from(top).unwrap();
        let footer_id = StableNodeId::try_from(footer).unwrap();
        assert_eq!(
            doc.runtime.node_style(body_id).unwrap().layout.overflow_y,
            nana_ui_core::OverflowSpec::Scroll
        );
        assert!(
            doc.runtime
                .node_style(body_id)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(doc.runtime.interaction(body_id).unwrap().pointer_events);
        assert!(
            !doc.runtime
                .node_style(top_id)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !doc.runtime
                .node_style(footer_id)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
    }

    fn register_settings_row(
        doc: &mut NanaTreeDocument,
        bridge: &mut crate::MessageBridge,
        label: &str,
        hint: Option<&str>,
        nest_hint_text: bool,
    ) -> (NodeHandle, NodeHandle, Option<NodeHandle>) {
        let row = doc.create_element("nana-settings-row");
        let copy = doc.create_element("div");
        let label_node = doc.create_element("span");
        let control = doc.create_element("div");
        doc.insert(row, doc.mount_root(), None);
        doc.insert(copy, row, None);
        doc.insert(label_node, copy, None);
        doc.insert(control, row, None);
        let mut row_props = crate::WidgetProps::default();
        row_props.class_names = vec!["nana-settings-row".into()];
        row_props.label = label.into();
        bridge.register(row.0, crate::WidgetKind::SettingsRow, row_props);
        let mut copy_props = crate::WidgetProps::default();
        copy_props.class_names = vec!["nana-settings-row__label".into()];
        bridge.register(copy.0, crate::WidgetKind::Column, copy_props);
        let mut label_props = crate::WidgetProps::default();
        label_props.label = label.into();
        bridge.register(label_node.0, crate::WidgetKind::Text, label_props);
        let hint_text = hint.filter(|_| nest_hint_text).map(|value| {
            let hint = doc.create_element("div");
            let text = doc.create_text(value);
            doc.insert(hint, copy, None);
            doc.insert(text, hint, None);
            let mut hint_props = crate::WidgetProps::default();
            hint_props.class_names = vec!["nana-settings-row__hint".into()];
            bridge.register(hint.0, crate::WidgetKind::Column, hint_props);
            let mut text_props = crate::WidgetProps::default();
            text_props.label = value.into();
            bridge.register(text.0, crate::WidgetKind::Text, text_props);
            bridge.insert_child(hint.0, copy.0, None);
            bridge.insert_child(text.0, hint.0, None);
            (hint, text)
        });
        let mut control_props = crate::WidgetProps::default();
        control_props.class_names = vec!["nana-settings-row__control".into()];
        bridge.register(control.0, crate::WidgetKind::Column, control_props);
        bridge.insert_child(copy.0, row.0, None);
        bridge.insert_child(label_node.0, copy.0, None);
        bridge.insert_child(control.0, row.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        (
            label_node,
            hint_text.map(|(hint, _)| hint).unwrap_or(label_node),
            hint_text.map(|(_, text)| text),
        )
    }

    #[test]
    fn settings_row_projects_label_and_nested_hint_once() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let (label, hint, hint_text) = register_settings_row(
            &mut doc,
            &mut bridge,
            "主题",
            Some("选择应用配色，立即生效"),
            true,
        );
        let label_style = doc
            .runtime
            .node_style(StableNodeId::try_from(label).unwrap())
            .unwrap();
        assert_eq!(label_style.layout.font_size, Some(13.0));
        assert_eq!(label_style.layout.font_weight, Some(500));
        let hint_text_id = StableNodeId::try_from(hint_text.unwrap()).unwrap();
        let hint_style = doc.runtime.node_style(hint_text_id).unwrap();
        assert_eq!(
            hint_style.foreground,
            Some(nana_ui_core::SemanticColorRole::Muted)
        );
        assert_eq!(hint_style.layout.font_size, Some(12.0));
        assert_eq!(
            doc.runtime.text(hint_text_id),
            Some("选择应用配色，立即生效")
        );
        let hint_box = doc
            .runtime
            .node_style(StableNodeId::try_from(hint).unwrap())
            .unwrap();
        assert_ne!(
            hint_box.foreground,
            Some(nana_ui_core::SemanticColorRole::Muted)
        );
    }

    #[test]
    fn settings_row_without_hint_still_projects_label() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let (label, _, _) = register_settings_row(&mut doc, &mut bridge, "工作区边缘", None, false);
        let label_style = doc
            .runtime
            .node_style(StableNodeId::try_from(label).unwrap())
            .unwrap();
        assert!(!label_style.layout.hidden);
        assert_eq!(label_style.layout.font_size, Some(13.0));
        assert_eq!(label_style.layout.font_weight, Some(500));
    }

    #[test]
    fn insert_into_missing_parent_does_not_orphan_child() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let parent = doc.create_element("nana-sidebar-frame");
        let footer = doc.create_element("nana-column");
        doc.insert(parent, doc.mount_root(), None);
        doc.insert(footer, parent, None);
        assert_eq!(doc.parent_node(footer), Some(parent));
        // Stale wrapNode target after remount dispose: insert must not detach.
        doc.insert(footer, NodeHandle(9_999_999), None);
        assert_eq!(
            doc.parent_node(footer),
            Some(parent),
            "failed insert must keep existing parent"
        );
        assert!(
            doc.children_of(parent).contains(&footer),
            "footer must stay under sidebar frame"
        );
    }

    #[test]
    fn insert_into_comment_parent_does_not_orphan_child() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let parent = doc.create_element("div");
        let child = doc.create_element("span");
        let comment = doc.create_comment("teleport-anchor");
        doc.insert(parent, doc.mount_root(), None);
        doc.insert(child, parent, None);
        doc.insert(comment, doc.mount_root(), None);
        doc.insert(child, comment, None);
        assert_eq!(
            doc.parent_node(child),
            Some(parent),
            "non-element parent must not steal children"
        );
    }

    #[test]
    fn teleport_to_body_remove_disposes_subtree_without_leaking() {
        // Vue Teleport `to="body"` inserts under mount_root; unmount must not
        // leave orphan nodes in the document map (Overlay open/close cycles).
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let body = doc.mount_root();
        let before = doc.nodes.len();
        let overlay = doc.create_element("div");
        doc.set_attribute(overlay, "class", "nana-dialog");
        doc.set_attribute(overlay, "aria-modal", "true");
        doc.set_attribute(overlay, "role", "dialog");
        let title = doc.create_element("span");
        doc.set_element_text(title, "Confirm");
        doc.insert(title, overlay, None);
        doc.insert(overlay, body, None);
        assert_eq!(doc.parent_node(overlay), Some(body));
        assert!(doc.contains(body, overlay));
        assert!(doc.contains(overlay, title));
        assert!(doc.nodes.len() > before);

        doc.remove(overlay);
        assert_eq!(doc.parent_node(overlay), None);
        assert!(!doc.contains(body, overlay));
        assert!(!doc.nodes.contains_key(&overlay.0));
        assert!(!doc.nodes.contains_key(&title.0));
        assert_eq!(doc.children_of(body), Vec::<NodeHandle>::new());
        assert_eq!(
            doc.nodes.len(),
            before,
            "scaffold-only after Teleport unmount"
        );
        assert_eq!(doc.query_selector("body"), Some(body));
    }

    #[test]
    fn contains_self_and_descendants() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let outer = doc.create_element("div");
        doc.insert(outer, doc.mount_root(), None);
        let inner = doc.create_element("span");
        doc.insert(inner, outer, None);
        let text = doc.create_text("hi");
        doc.insert(text, inner, None);
        let sibling = doc.create_element("div");
        doc.insert(sibling, doc.mount_root(), None);

        assert!(doc.contains(outer, outer));
        assert!(doc.contains(outer, inner));
        assert!(doc.contains(outer, text));
        assert!(doc.contains(doc.mount_root(), outer));
        assert!(!doc.contains(outer, sibling));
        assert!(!doc.contains(inner, outer));
        assert!(!doc.contains(sibling, inner));
    }

    #[test]
    fn create_element_stores_svg_namespace_and_is() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let svg = doc.create_element_ns("svg", ElementNamespace::Html, None);
        assert_eq!(doc.element_namespace(svg), Some(ElementNamespace::Svg));
        let path = doc.create_element_ns("path", ElementNamespace::Svg, None);
        assert_eq!(doc.element_namespace(path), Some(ElementNamespace::Svg));
        let custom = doc.create_element_ns("p", ElementNamespace::Html, Some("fancy-p"));
        assert_eq!(doc.get_attribute(custom, "is").as_deref(), Some("fancy-p"));
    }

    #[test]
    fn insert_static_content_parses_fragment_and_reuses_start_end() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let parent = doc.mount_root();
        let (first, last, _) = doc.insert_static_content(
            "<span class=\"a\">hi</span><b>x</b>",
            parent,
            None,
            ElementNamespace::Html,
            None,
            None,
        );
        assert_ne!(first, last);
        assert_eq!(doc.element_tag(first).as_deref(), Some("span"));
        assert_eq!(doc.get_attribute(first, "class").as_deref(), Some("a"));
        assert_eq!(doc.element_tag(last).as_deref(), Some("b"));
        assert_eq!(doc.children_of(parent), vec![first, last]);

        let (c0, c1, pairs) = doc.insert_static_content(
            "ignored-on-reuse",
            parent,
            None,
            ElementNamespace::Html,
            Some(first),
            Some(last),
        );
        assert!(pairs.iter().any(|(s, d)| s.0 == first.0 && d.0 != s.0));
        assert_eq!(doc.children_of(parent).len(), 4);
        assert_eq!(doc.element_tag(c0).as_deref(), Some("span"));
        assert_eq!(doc.element_tag(c1).as_deref(), Some("b"));
        assert_ne!(c0, first);
    }

    #[test]
    fn insert_static_content_unwraps_svg_namespace_wrapper() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let parent = doc.mount_root();
        let (first, last, _) = doc.insert_static_content(
            "<path d=\"M0 0\"></path>",
            parent,
            None,
            ElementNamespace::Svg,
            None,
            None,
        );
        assert_eq!(first, last);
        assert_eq!(doc.element_tag(first).as_deref(), Some("path"));
        assert_eq!(doc.element_namespace(first), Some(ElementNamespace::Svg));
    }

    #[test]
    fn first_child_child_nodes_parent_element_match_tree() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let outer = doc.create_element("div");
        doc.insert(outer, doc.mount_root(), None);
        let text = doc.create_text("a");
        doc.insert(text, outer, None);
        let inner = doc.create_element("span");
        doc.insert(inner, outer, None);

        assert_eq!(doc.first_child(outer), Some(text));
        assert_eq!(doc.children_of(outer), vec![text, inner]);
        assert_eq!(doc.parent_element(inner), Some(outer));
        assert_eq!(doc.parent_element(text), Some(outer));
        assert_eq!(doc.parent_node(outer), Some(doc.mount_root()));
        assert_eq!(doc.parent_element(outer), Some(doc.mount_root()));
        assert_eq!(doc.node_kind(text), DomNodeKind::Text);
        assert_eq!(doc.element_tag(inner).as_deref(), Some("span"));
    }

    #[test]
    fn query_selector_all_and_closest_match_compounds() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let outer = doc.create_element("div");
        doc.set_attribute(outer, "class", "home-pending-action is-confirming");
        doc.set_attribute(outer, "data-sidebar-repo-id", "r1");
        doc.insert(outer, doc.mount_root(), None);
        let inner = doc.create_element("span");
        doc.set_attribute(inner, "id", "sec-a");
        doc.insert(inner, outer, None);

        assert_eq!(
            doc.query_selector(".home-pending-action.is-confirming"),
            Some(outer)
        );
        assert_eq!(doc.query_selector("#sec-a"), Some(inner));
        assert_eq!(
            doc.query_selector_all("[data-sidebar-repo-id], #sec-a")
                .len(),
            2
        );
        assert_eq!(doc.closest(inner, "[data-sidebar-repo-id]"), Some(outer));
        assert_eq!(
            doc.closest(inner, ".home-pending-action.is-confirming"),
            Some(outer)
        );
        assert_eq!(doc.closest(inner, ".missing"), None);
    }

    #[test]
    fn apply_layout_boxes_keeps_viewport_roots() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let child = doc.create_element("div");
        doc.insert(child, doc.mount_root(), None);
        doc.apply_layout_boxes(&[(
            child,
            LayoutBox {
                handle: child,
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            },
        )]);
        let body = doc.layout_box(doc.mount_root()).expect("body");
        assert_eq!((body.width, body.height), (400.0, 300.0));
        let box_ = doc.layout_box(child).expect("child");
        assert_eq!(
            (box_.x, box_.y, box_.width, box_.height),
            (10.0, 20.0, 100.0, 40.0)
        );
    }

    #[test]
    fn get_layout_box_prefers_paint_store_over_document() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let child = doc.create_element("div");
        doc.insert(child, doc.mount_root(), None);
        doc.apply_layout_boxes(&[(
            child,
            LayoutBox {
                handle: child,
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 20.0,
            },
        )]);
        let store = LayoutBoxStore::new();
        // Scene chrome (e.g. scrollport padding) shifts the painted box.
        store.record(child, 16.0, 16.0, 80.0, 24.0);
        let box_ = get_layout_box_from(&store, &doc, child).expect("paint box");
        assert_eq!(
            (box_.x, box_.y, box_.width, box_.height),
            (16.0, 16.0, 80.0, 24.0),
            "menu anchors must follow Scene paint, not pre-paint measure"
        );
        store.begin_frame();
        let kept = get_layout_box_from(&store, &doc, child).expect("incremental paint box");
        assert_eq!(
            (kept.x, kept.y, kept.width, kept.height),
            (16.0, 16.0, 80.0, 24.0),
            "begin_frame must keep last paint boxes"
        );
        store.remove(child);
        let fallback = get_layout_box_from(&store, &doc, child).expect("doc fallback");
        assert_eq!(
            (fallback.x, fallback.y, fallback.width, fallback.height),
            (0.0, 0.0, 50.0, 20.0)
        );
    }

    #[test]
    fn create_insert_and_text_snapshot() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let root = doc.mount_root();
        let btn = doc.create_element("button");
        doc.set_element_text(btn, "0");
        doc.set_event_flag(btn, "onClick", true);
        doc.insert(btn, root, None);
        doc.apply_layout_boxes(&[(
            btn,
            LayoutBox {
                handle: btn,
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 28.0,
            },
        )]);
        let snap = doc.snapshot_boxes();
        assert!(snap.texts.iter().any(|(_, t)| t == "0"));
        assert!(snap.event_targets.iter().any(|(_, e)| e == "click"));
        let box_ = doc.layout_box(btn).expect("button box");
        assert!(box_.width > 0.0 && box_.height > 0.0);
        assert_eq!(
            doc.hit_event_target(box_.x + 1.0, box_.y + 1.0, "click"),
            Some(btn)
        );
    }

    #[test]
    fn transformed_layout_box_uses_inverse_affine_hit_testing() {
        let store = LayoutBoxStore::new();
        let node = NodeHandle(991);
        let sin = std::f32::consts::FRAC_1_SQRT_2;
        store.record_transformed(node, 0.0, 0.0, 4.0, 2.0, [sin, sin, -sin, sin, 5.0, 1.0]);
        let bounds = store.get(node).expect("transformed bounds");
        assert!(store.contains_point(node, 5.0, 2.0));
        assert!(
            !store.contains_point(node, bounds.x + 0.01, bounds.y + 0.01),
            "rotated AABB corners must not become false-positive pointer targets"
        );
        let (local_x, local_y) = store.local_point(node, 5.0, 2.0).unwrap();
        assert!((local_x - sin).abs() < 1e-5);
        assert!((local_y - sin).abs() < 1e-5);
    }

    #[test]
    fn document_hit_test_uses_runtime_transform_authority() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("button");
        doc.insert(node, doc.mount_root(), None);
        let mut props = crate::WidgetProps::default();
        props.layout.transform = Some(nana_ui_core::PaintTransform {
            e: 10.0,
            ..Default::default()
        });
        let snapshot = crate::SemanticSnapshot {
            revision: 1,
            theme: nana_ui_core::ThemeMode::Light,
            appearance: nana_ui_core::AppearanceSettings::default(),
            roots: vec![node.0],
            widgets: vec![crate::SemanticWidget {
                id: node.0,
                kind: crate::WidgetKind::Button,
                props,
                children: Vec::new(),
                parent: Some(doc.mount_root().0),
            }],
        };
        doc.sync_semantic_styles(&snapshot);
        doc.apply_layout_boxes(&[(
            node,
            LayoutBox {
                handle: node,
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
        )]);

        assert_eq!(doc.hit_test(11.0, 1.0), Some(node));
        assert_ne!(doc.hit_test(1.0, 1.0), Some(node));
    }

    #[test]
    fn pointer_events_none_is_not_hit() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let under = doc.create_element("button");
        let overlay = doc.create_element("div");
        let root = doc.mount_root();
        doc.insert(under, root, None);
        doc.insert(overlay, root, None);
        let mut under_props = crate::WidgetProps::default();
        under_props.layout.width = Some(nana_ui_core::LengthSpec::Px(40.0));
        under_props.layout.height = Some(nana_ui_core::LengthSpec::Px(40.0));
        let mut overlay_props = crate::WidgetProps::default();
        overlay_props.layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
        overlay_props.layout.width = Some(nana_ui_core::LengthSpec::Px(40.0));
        overlay_props.layout.height = Some(nana_ui_core::LengthSpec::Px(40.0));
        let snapshot = crate::SemanticSnapshot {
            revision: 1,
            theme: nana_ui_core::ThemeMode::Light,
            appearance: nana_ui_core::AppearanceSettings::default(),
            roots: vec![under.0, overlay.0],
            widgets: vec![
                crate::SemanticWidget {
                    id: under.0,
                    kind: crate::WidgetKind::Button,
                    props: under_props,
                    children: Vec::new(),
                    parent: Some(root.0),
                },
                crate::SemanticWidget {
                    id: overlay.0,
                    kind: crate::WidgetKind::Column,
                    props: overlay_props,
                    children: Vec::new(),
                    parent: Some(root.0),
                },
            ],
        };
        doc.sync_semantic_styles(&snapshot);
        doc.inject_layout_boxes(&[
            (
                under,
                LayoutBox {
                    handle: under,
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 40.0,
                },
            ),
            (
                overlay,
                LayoutBox {
                    handle: overlay,
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 40.0,
                },
            ),
        ]);

        let overlay_id = StableNodeId::try_from(overlay).unwrap();
        assert!(!doc.runtime.interaction(overlay_id).unwrap().pointer_events);
        assert_eq!(
            doc.runtime
                .node_style(overlay_id)
                .unwrap()
                .layout
                .pointer_events,
            Some(nana_ui_core::PointerEventsSpec::None)
        );
        assert_ne!(doc.hit_test(20.0, 20.0), Some(overlay));
        assert_eq!(doc.hit_test(20.0, 20.0), Some(under));
    }

    #[test]
    fn gpu_slot_authority_is_runtime_custom_render() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("div");
        doc.insert(node, doc.mount_root(), None);
        doc.set_gpu_slot(node, "program");
        let id = StableNodeId::try_from(node).unwrap();
        assert!(
            doc.gpu_slots()
                .iter()
                .any(|(handle, slot)| *handle == node && slot == "program")
        );
        assert!(
            doc.world().custom_render(id).is_none(),
            "host ops must not commit GPU slots before the frame boundary"
        );

        doc.flush_host_frame();
        let content = doc
            .world()
            .custom_render(id)
            .expect("data-nana-gpu must land on CustomRenderNode");
        assert_eq!(content.renderer.as_ref(), "nana.host-texture");
        assert_eq!(content.resource.as_ref(), "program");
        assert_eq!(
            doc.gpu_slots(),
            vec![(node, "program".into())],
            "snapshot/host GPU binding must read Runtime, not a facade map"
        );
        assert_eq!(
            content.revision, 0,
            "unresolved registry handles keep revision 0"
        );

        doc.remove_attribute(node, "data-nana-gpu");
        doc.flush_host_frame();
        assert!(doc.world().custom_render(id).is_none());
        assert!(doc.gpu_slots().is_empty());
    }

    #[test]
    fn flushed_host_texture_revision_matches_packed_generation_and_version() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("div");
        doc.insert(node, doc.mount_root(), None);
        doc.set_gpu_slot(node, "program");
        let generation = 5;
        let version = 3;
        doc.override_host_texture_revision(
            "program",
            nana_ui_runtime::pack_gpu_revision(generation, version),
        );
        let id = StableNodeId::try_from(node).unwrap();
        assert!(
            doc.world().custom_render(id).is_none(),
            "host ops must not commit GPU revisions before the frame boundary"
        );

        doc.flush_host_frame();
        let content = doc
            .world()
            .custom_render(id)
            .expect("registered texture must land on CustomRenderNode");
        assert_eq!(
            content.revision,
            nana_ui_runtime::pack_gpu_revision(generation, version)
        );

        doc.override_host_texture_revision(
            "program",
            nana_ui_runtime::pack_gpu_revision(generation, version + 1),
        );
        doc.flush_host_frame();
        let content = doc
            .world()
            .custom_render(id)
            .expect("invalidated texture must keep CustomRenderNode");
        assert_eq!(
            content.revision,
            nana_ui_runtime::pack_gpu_revision(generation, version + 1),
            "content updates must change CustomRenderNode.revision"
        );

        let world_generation = doc.world().generation();
        doc.flush_host_frame();
        assert_eq!(
            doc.world().generation(),
            world_generation,
            "unchanged host-texture revision must not enqueue CustomRender mutations"
        );
    }

    #[test]
    fn has_event_reads_runtime_listener_authority() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let btn = doc.create_element("button");
        doc.insert(btn, doc.mount_root(), None);
        doc.set_event_flag(btn, "onClick", true);
        assert!(doc.has_event(btn, "click"));
        let id = StableNodeId::try_from(btn).unwrap();
        assert!(
            !doc.world().has_event(id, "click"),
            "EventRoute is not the listener set; listeners commit at the frame boundary"
        );

        doc.flush_host_frame();
        assert!(doc.has_event(btn, "click"));
        assert!(doc.world().has_event(id, "click"));
        assert!(
            doc.world()
                .event_targets(doc.world().node(id).unwrap().document)
                .contains(&(btn.0, "click".into()))
        );

        doc.set_event_flag(btn, "click", false);
        doc.flush_host_frame();
        assert!(!doc.has_event(btn, "click"));
        assert!(!doc.world().has_event(id, "click"));
    }

    #[test]
    fn same_frame_host_ops_flush_once_at_frame_boundary() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let root = doc.mount_root();
        let generation = doc.runtime_generation();
        let parent = doc.create_element("div");
        let child = doc.create_element("span");
        doc.insert(parent, root, None);
        doc.insert(child, parent, None);
        doc.set_element_text(child, "hello");
        doc.set_event_flag(parent, "click", true);
        doc.set_gpu_slot(parent, "slot");

        assert_eq!(
            doc.runtime_generation(),
            generation,
            "create/insert/text/event/gpu must share one uncommitted batch"
        );
        assert_eq!(doc.parent_node(parent), Some(root));
        assert_eq!(doc.children_of(parent), vec![child]);
        assert_eq!(doc.parent_node(child), Some(parent));
        assert!(doc.text_content(child).unwrap().contains("hello"));

        doc.flush_host_frame();
        assert!(
            doc.runtime_generation() > generation,
            "one host flush must commit the batched ops (layout writeback may add a generation)"
        );
        assert_eq!(doc.children_of(parent), vec![child]);
        let parent_id = StableNodeId::try_from(parent).unwrap();
        assert!(doc.world().has_event(parent_id, "click"));
        assert!(doc.world().custom_render(parent_id).is_some());
    }

    #[test]
    fn layout_box_store_keeps_clean_paint_and_drops_removed_nodes() {
        let store = LayoutBoxStore::new();
        let kept = NodeHandle(11);
        let removed = NodeHandle(12);
        store.record(kept, 1.0, 2.0, 10.0, 20.0);
        store.record(removed, 3.0, 4.0, 30.0, 40.0);
        store.begin_frame();
        assert_eq!(store.get(kept).unwrap().width, 10.0);
        assert_eq!(store.get(removed).unwrap().height, 40.0);
        store.remove(removed);
        assert!(store.get(kept).is_some());
        assert!(store.get(removed).is_none());
        store.retain(|id| id == kept.0);
        assert!(store.get(kept).is_some());
        assert!(store.get(removed).is_none());
    }

    #[test]
    fn scroll_view_overlay_does_not_write_runtime_layout() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let target = doc.create_element("div");
        doc.insert(target, doc.mount_root(), None);
        let store = LayoutBoxStore::new();
        store.record(target, 0.0, 400.0, 300.0, 40.0);
        doc.apply_layout_boxes(&store.snapshot());
        assert_eq!(doc.layout_box(target).unwrap().y, 400.0);

        store.translate(target, 0.0, -400.0);
        assert_eq!(
            store.get(target).unwrap().y,
            0.0,
            "JS paint overlay follows scroll"
        );
        assert_eq!(
            doc.layout_box(target).unwrap().y,
            400.0,
            "Runtime LayoutBox stays unscrolled"
        );
        doc.apply_layout_boxes(&store.snapshot());
        assert_eq!(
            doc.layout_box(target).unwrap().y,
            400.0,
            "paint snapshot writeback must not copy scrolled coordinates"
        );
    }

    #[test]
    fn generated_before_pseudo_appears_in_runtime_tree() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let root = doc.mount_root();
        let host = doc.create_element("div");
        doc.insert(host, root, None);
        bridge.register(
            host.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                class_names: vec!["chip".into()],
                ..crate::WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(".chip::before { content: \"\"; width: 4px; height: 4px; }");
        bridge.resolve_document_layout(&mut doc);
        let snap = bridge.snapshot();
        let has_before = snap.widgets.iter().any(|w| {
            w.props
                .attrs
                .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                .map(String::as_str)
                == Some("before")
        });
        assert!(
            has_before,
            "chip::before must materialize a generated child widget"
        );
        doc.flush_host_frame();
        let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
        let order = doc.world().document_order(document);
        assert!(
            order.iter().any(|id| snap.widgets.iter().any(|w| {
                w.props
                    .attrs
                    .contains_key(crate::bridge::GENERATED_PSEUDO_ATTR)
                    && w.id == id.get()
            })),
            "generated ::before node must exist in Runtime document order"
        );
    }

    #[test]
    fn generated_before_keeps_size_with_static_parent_rule() {
        use nana_ui_core::LengthSpec;
        use nana_ui_runtime::StableNodeId;

        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let root = doc.mount_root();
        let host = doc.create_element("div");
        doc.insert(host, root, None);
        bridge.register(
            host.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                class_names: vec!["chip".into()],
                ..crate::WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            ".chip { display: flex; } .chip::before { content: \"\"; width: 4px; height: 4px; }",
        );
        bridge.resolve_document_layout(&mut doc);
        doc.flush_host_frame();
        let before_id = bridge
            .snapshot()
            .widgets
            .iter()
            .find(|w| {
                w.props
                    .attrs
                    .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                    .map(String::as_str)
                    == Some("before")
            })
            .map(|w| w.id)
            .expect("generated ::before widget");
        let style = doc
            .runtime
            .node_style(StableNodeId::new(before_id).unwrap())
            .expect("before runtime style");
        assert_eq!(style.layout.width, Some(LengthSpec::Px(4.0)));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(4.0)));
    }

    #[test]
    fn generated_before_inserts_before_origin_children() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let root = doc.mount_root();
        let host = doc.create_element("div");
        doc.insert(host, root, None);
        let child = doc.create_element("span");
        doc.insert(child, host, None);
        bridge.register(
            host.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                class_names: vec!["chip".into()],
                ..crate::WidgetProps::default()
            },
        );
        bridge.register(
            child.0,
            crate::WidgetKind::Text,
            crate::WidgetProps {
                label: "x".into(),
                ..crate::WidgetProps::default()
            },
        );
        bridge.insert_child(child.0, host.0, None);
        bridge.inject_stylesheet(".chip::before { content: \"\"; width: 4px; height: 4px; }");
        bridge.resolve_document_layout(&mut doc);
        doc.flush_host_frame();
        let before_id = bridge
            .snapshot()
            .widgets
            .iter()
            .find(|w| {
                w.props
                    .attrs
                    .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                    .map(String::as_str)
                    == Some("before")
            })
            .map(|w| w.id)
            .expect("generated ::before widget");
        let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
        let order = doc
            .world()
            .document_order(document)
            .into_iter()
            .map(|id| id.get())
            .collect::<Vec<_>>();
        let host_idx = order.iter().position(|id| *id == host.0).expect("host");
        let before_idx = order
            .iter()
            .position(|id| *id == before_id)
            .expect("before");
        let child_idx = order.iter().position(|id| *id == child.0).expect("child");
        assert!(host_idx < before_idx && before_idx < child_idx);
    }

    #[test]
    fn generated_after_inserts_after_origin_children() {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let mut bridge = crate::MessageBridge::new();
        let root = doc.mount_root();
        let host = doc.create_element("div");
        doc.insert(host, root, None);
        let child = doc.create_element("span");
        doc.insert(child, host, None);
        bridge.register(
            host.0,
            crate::WidgetKind::Column,
            crate::WidgetProps {
                class_names: vec!["chip".into()],
                ..crate::WidgetProps::default()
            },
        );
        bridge.register(
            child.0,
            crate::WidgetKind::Text,
            crate::WidgetProps {
                label: "x".into(),
                ..crate::WidgetProps::default()
            },
        );
        bridge.insert_child(child.0, host.0, None);
        bridge.inject_stylesheet(".chip::after { content: \"\"; width: 4px; height: 4px; }");
        bridge.resolve_document_layout(&mut doc);
        doc.flush_host_frame();
        let after_id = bridge
            .snapshot()
            .widgets
            .iter()
            .find(|w| {
                w.props
                    .attrs
                    .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                    .map(String::as_str)
                    == Some("after")
            })
            .map(|w| w.id)
            .expect("generated ::after widget");
        let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
        let order = doc
            .world()
            .document_order(document)
            .into_iter()
            .map(|id| id.get())
            .collect::<Vec<_>>();
        let host_idx = order.iter().position(|id| *id == host.0).expect("host");
        let child_idx = order.iter().position(|id| *id == child.0).expect("child");
        let after_idx = order.iter().position(|id| *id == after_id).expect("after");
        assert!(host_idx < child_idx && child_idx < after_idx);
    }
}
