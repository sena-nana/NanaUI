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
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
    time::Instant,
};

#[cfg(any(test, feature = "hosted"))]
use nana_ui_runtime::AccessibilityUpdate;
#[cfg(feature = "graph-canvas")]
use nana_ui_runtime::GraphCanvas as RuntimeGraphCanvas;
#[cfg(not(feature = "scene-view"))]
use nana_ui_runtime::MeasureTextShaper;
#[cfg(all(test, feature = "rich-text"))]
use nana_ui_runtime::NativeMarkdown as RuntimeNativeMarkdown;
#[cfg(feature = "graph-canvas")]
use nana_ui_runtime::RegisterableComponent;
use nana_ui_runtime::{
    AccessibilityDelta, AccessibilityRole, AccessibilityState, AppContext,
    AppShell as RuntimeAppShell, AppTitleBar as RuntimeAppTitleBar, ComponentBindKind,
    ComponentTypeId, ComponentView, CustomRenderNode, Dock as RuntimeDock, DockAxis, DockNode,
    Entity, HOST_TEXTURE_RENDERER, ImeComposition, InteractionState, LayoutBox as RuntimeLayoutBox,
    LayoutViewport, MutationQueue, NodeKind, NodeStyle, SegmentedOption as RuntimeSegmentedOption,
    SelectionChrome, SemanticOption, SemanticSpec, SettingsPage as RuntimeSettingsPage,
    SidebarFrame as RuntimeSidebarFrame, SplitPane as RuntimeSplitPane, StableNodeId, TextContent,
    TextInputState, UiMutation, UiWorld, Workspace as RuntimeWorkspace, WorkspaceRegionSlot,
};
use nana_ui_scene::{RuntimeDocument, UiScene};

mod component_binding;
mod gpu_slots;
mod kits;
mod layout;
pub(crate) mod semantic_read;
use semantic_read::{PreparedSemanticSync, SemanticRead, SemanticWidgetView};

pub(crate) use component_binding::*;
pub(crate) use gpu_slots::*;
pub(crate) use kits::*;

pub use layout::{
    BoxSnapshot, DomNodeKind, LayoutBox, LayoutBoxStore, get_layout_box, get_layout_box_from,
    query_scroll_content_size,
};

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

/// Vue-owned Runtime. World reads/writes forward to [`UiWorld`]; typed views
/// and `assemble_*` live on [`AppContext`]. The retained scene lives on the
/// same [`RuntimeDocument`] the Scene host flushes.
struct VueRuntime {
    document: RuntimeDocument,
}

impl VueRuntime {
    fn new(document: nana_ui_runtime::DocumentId) -> Self {
        Self {
            document: RuntimeDocument::new(document),
        }
    }

    fn world(&self) -> &UiWorld {
        self.document.context().world()
    }

    fn context(&self) -> &AppContext {
        self.document.context()
    }

    fn context_mut(&mut self) -> &mut AppContext {
        self.document.context_mut()
    }

    fn runtime_document(&self) -> &RuntimeDocument {
        &self.document
    }

    fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }
}

impl std::ops::Deref for VueRuntime {
    type Target = UiWorld;

    fn deref(&self) -> &UiWorld {
        self.document.context().world()
    }
}

impl std::ops::DerefMut for VueRuntime {
    fn deref_mut(&mut self) -> &mut UiWorld {
        self.document.context_mut().world_mut()
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
        UiMutation::SetStyleTokens { .. } => "SetStyleTokens",
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
        UiMutation::SetSurfaceOpen { .. } => "SetSurfaceOpen",
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
        UiMutation::SetTextInputFoldCollapsed { .. } => "SetTextInputFoldCollapsed",
        UiMutation::SetTextInputSnippet { .. } => "SetTextInputSnippet",
        UiMutation::SetTextInputCompletions { .. } => "SetTextInputCompletions",
        UiMutation::SetTextInputCompletionView { .. } => "SetTextInputCompletionView",
        UiMutation::SetTextInputCompletionDismissed { .. } => "SetTextInputCompletionDismissed",
        UiMutation::SetTextInputCompletionReopened { .. } => "SetTextInputCompletionReopened",
        UiMutation::SetTextInputHover { .. } => "SetTextInputHover",
        UiMutation::SetTextInputHoverScroll { .. } => "SetTextInputHoverScroll",
    }
}

#[derive(Default)]
pub(crate) struct PendingAssembly {
    bindings: Vec<nana_ui_runtime::PreparedSemanticBinding>,
    workspaces: Vec<(StableNodeId, RuntimeWorkspace)>,
    docks: Vec<(StableNodeId, RuntimeDock)>,
    split_panes: Vec<(StableNodeId, RuntimeSplitPane)>,
    title_bars: Vec<(StableNodeId, RuntimeAppTitleBar)>,
    app_shells: Vec<(StableNodeId, RuntimeAppShell)>,
    settings_pages: Vec<(StableNodeId, RuntimeSettingsPage)>,
    #[cfg(feature = "graph-canvas")]
    graph_canvases: Vec<(StableNodeId, RuntimeGraphCanvas)>,
}

impl PendingAssembly {
    fn apply(self, context: &mut AppContext) {
        for binding in self.bindings {
            // The matching projection has already been committed.
            let _ = context.finish_semantic_binding(binding);
        }
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
        for (id, component) in self.settings_pages {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_settings_page(entity);
            }
        }
        #[cfg(feature = "graph-canvas")]
        for (id, component) in self.graph_canvases {
            if let Ok(entity) = context.bind_component(id, component) {
                let _ = context.assemble_graph_canvas_contents(entity);
            }
        }
    }
}

/// Vue custom-renderer facade. Not the product retained world.
///
/// Identity, hierarchy, kind, text, focus, style, interaction, and layout live
/// in `nana_ui_runtime::UiWorld`. `nodes` keeps only Vue DOM metadata
/// (namespace, attributes, scope id) needed by host ops. Do not treat this type
/// as a second ECS/DOM tree.
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
    /// Cached generic SVG rasters keyed by the root `<svg>` node id.
    svg_rasters: HashMap<u64, CachedSvgRaster>,
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
            svg_rasters: HashMap::new(),
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
        let mut surface_ids = Vec::new();
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
            if self.host_texture_nodes.contains(&raw_id) {
                surface_ids.push(raw_id);
            }
        }
        if !mutations.is_empty() {
            self.commit_extra(mutations).ok();
        }
        if !surface_ids.is_empty() {
            for raw_id in surface_ids {
                self.sync_surface_custom_render(NodeHandle(raw_id));
            }
            self.commit_pending_queue().ok();
        }
    }

    /// Commit queued Vue host ops, then drain Runtime systems.
    pub fn flush_host_frame(&mut self) {
        self.sync_svg_rasters();
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

    /// Rasterize non-Lucide `<svg>` roots into HostTexture slots.
    pub(crate) fn sync_svg_rasters(&mut self) {
        let roots: Vec<u64> = self
            .nodes
            .keys()
            .copied()
            .filter(|&id| {
                self.element_tag(NodeHandle(id))
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("svg"))
            })
            .collect();
        for id in roots {
            let el = NodeHandle(id);
            if self.is_catalog_icon_svg(el) {
                if self.svg_rasters.remove(&id).is_some() {
                    self.index_host_texture_node(el);
                    self.sync_surface_custom_render(el);
                }
                continue;
            }
            let Some(element) = self.collect_svg_element(el) else {
                continue;
            };
            let markup = crate::svg_raster::serialize_svg(&element);
            let markup_hash = hash_svg_markup(&markup);
            let (width, height) = self.svg_raster_size(el);
            if width == 0 || height == 0 {
                continue;
            }
            let unchanged = self.svg_rasters.get(&id).is_some_and(|cached| {
                cached.markup_hash == markup_hash
                    && cached.pixel_width == width
                    && cached.pixel_height == height
            });
            if !unchanged {
                let Some(raster) = crate::svg_raster::rasterize_svg(&markup, width, height) else {
                    continue;
                };
                let version = self
                    .svg_rasters
                    .get(&id)
                    .map(|cached| cached.version.saturating_add(1))
                    .unwrap_or(1);
                self.svg_rasters.insert(
                    id,
                    CachedSvgRaster {
                        markup_hash,
                        pixel_width: width,
                        pixel_height: height,
                        raster,
                        version,
                    },
                );
            }
            self.index_host_texture_node(el);
            self.sync_surface_custom_render(el);
        }
    }

    #[cfg_attr(not(any(test, feature = "hosted")), allow(dead_code))]
    pub(crate) fn svg_host_uploads(&self) -> Vec<crate::svg_raster::SvgHostUpload> {
        self.svg_rasters
            .iter()
            .map(|(&id, cached)| crate::svg_raster::SvgHostUpload {
                slot: format!("svg:{id}"),
                node: id,
                raster: cached.raster.clone(),
                version: cached.version,
            })
            .collect()
    }

    fn is_catalog_icon_svg(&self, el: NodeHandle) -> bool {
        self.get_attribute(el, "class")
            .or_else(|| self.get_attribute(el, "className"))
            .is_some_and(|class| {
                class
                    .split_whitespace()
                    .any(|token| nana_ui_core::Icon::parse_name(token).is_some())
            })
    }

    fn svg_raster_size(&self, el: NodeHandle) -> (u32, u32) {
        let scale = self.scale_factor.max(0.01);
        if let Some(box_) = self.layout_box(el) {
            let width = (box_.width * scale).round() as u32;
            let height = (box_.height * scale).round() as u32;
            if width > 0 && height > 0 {
                return (width.min(2048), height.min(2048));
            }
        }
        parse_svg_intrinsic_size(
            self.get_attribute(el, "width").as_deref(),
            self.get_attribute(el, "height").as_deref(),
            self.get_attribute(el, "viewBox").as_deref(),
        )
    }

    fn collect_svg_element(&self, el: NodeHandle) -> Option<crate::svg_raster::SvgElement> {
        let tag = self.element_tag(el)?;
        let attrs = match self.nodes.get(&el.0).map(|node| &node.data) {
            Some(NodeData::Element { attrs, .. }) => attrs
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut children = Vec::new();
        for child in self.children_of(el) {
            match self.node_kind(child) {
                DomNodeKind::Element => {
                    if let Some(element) = self.collect_svg_element(child) {
                        children.push(crate::svg_raster::SvgNode::Element(element));
                    }
                }
                DomNodeKind::Text => {
                    if let Some(text) = self.runtime_text(child).filter(|text| !text.is_empty()) {
                        children.push(crate::svg_raster::SvgNode::Text(text));
                    }
                }
                _ => {}
            }
        }
        Some(crate::svg_raster::SvgElement {
            tag,
            attrs,
            children,
        })
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
        let previous_parent = self.live_parent(child);
        if let Some(old_parent) = previous_parent
            && old_parent != parent
        {
            self.overlay_children_mut(old_parent)
                .retain(|id| *id != child);
        }
        self.pending.parent.insert(child, Some(parent));
        let siblings = self.overlay_children_mut(parent);
        if previous_parent == Some(parent) {
            siblings.retain(|id| *id != child);
        }
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
        let prepared = self.prepare_semantic_styles(&SemanticRead::snapshot(snapshot));
        self.apply_semantic_styles(prepared);
    }

    /// Apply changed semantic props by borrowing the bridge; consume its mutation footprint.
    pub fn sync_semantics_from_bridge(&mut self, bridge: &mut crate::MessageBridge) {
        if self.synced_semantic_revision == Some(bridge.revision()) {
            if !self.pending.is_empty() {
                self.flush_host_frame();
            }
            return;
        }
        self.flush_host_frame();
        let changes = bridge.take_snapshot_changes();
        let prepared = self.prepare_semantic_styles(&SemanticRead::bridge(bridge, self, changes));
        self.apply_semantic_styles(prepared);
    }

    fn prepare_semantic_styles(&self, snapshot: &SemanticRead<'_>) -> PreparedSemanticSync {
        // Incremental pass: project only the widgets the bridge marked dirty.
        // A structural change, a whole-document invalidation, or an empty
        // footprint with a bumped revision (footprint drained by another
        // consumer) falls back to the full preorder pass.
        let full_pass = snapshot.changes.needs_full_pass() || snapshot.changes.dirty.is_empty();
        let mut mutations = MutationQueue::new();
        let mut pending = PendingAssembly::default();
        let mut component_owned_layout = HashSet::new();
        if self.runtime.theme_mode() != snapshot.theme {
            mutations.set_theme(snapshot.theme);
        }
        let projected = snapshot.projection_ids(full_pass);
        for raw_id in &projected {
            let Some(widget) = snapshot.get(*raw_id) else {
                continue;
            };
            let widget = &widget;
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
                        | crate::WidgetKind::NumberInput
                        | crate::WidgetKind::Textarea
                        | crate::WidgetKind::ContextMenu
                        | crate::WidgetKind::CommandPalette
                ))
                && matches!(
                    widget.kind,
                    crate::WidgetKind::Button
                        | crate::WidgetKind::IconButton
                        | crate::WidgetKind::Checkbox
                        | crate::WidgetKind::Radio
                        | crate::WidgetKind::Switch
                        | crate::WidgetKind::NumberInput
                        | crate::WidgetKind::Card
                        | crate::WidgetKind::Divider
                        | crate::WidgetKind::Thumbnail
                        | crate::WidgetKind::List
                        | crate::WidgetKind::ListItem
                        | crate::WidgetKind::ScrollView
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
                        | crate::WidgetKind::DesktopShell
                        | crate::WidgetKind::AppTitleBar
                        | crate::WidgetKind::PaneChrome
                        | crate::WidgetKind::SidebarSection
                        | crate::WidgetKind::SidebarFooter
                        | crate::WidgetKind::SettingsPage
                        | crate::WidgetKind::SettingsCollapsibleCard
                        | crate::WidgetKind::Table
                        | crate::WidgetKind::TableRow
                        | crate::WidgetKind::TableCell
                        | crate::WidgetKind::ReorderList
                        | crate::WidgetKind::TimeSeriesChart
                        | crate::WidgetKind::GpuTextureView
                        | crate::WidgetKind::GpuView
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
                if widget.kind == crate::WidgetKind::GraphCanvas {
                    for child in widget.children.iter() {
                        component_owned_layout.insert(*child);
                    }
                }
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
                                | crate::WidgetKind::IconButton
                                | crate::WidgetKind::Chip
                                | crate::WidgetKind::Input
                                | crate::WidgetKind::NumberInput
                                | crate::WidgetKind::Textarea
                                | crate::WidgetKind::Checkbox
                                | crate::WidgetKind::Radio
                                | crate::WidgetKind::Switch
                                | crate::WidgetKind::Tabs
                                | crate::WidgetKind::Segmented
                                | crate::WidgetKind::Range
                                | crate::WidgetKind::ListItem
                                | crate::WidgetKind::TableRow
                                | crate::WidgetKind::InteractiveCard
                        )),
            };
            if self.runtime.interaction(id) != Some(interaction) {
                mutations.set_interaction(id, interaction);
            }
            let accessible_name = self.semantic_accessible_name(widget);
            let accessibility = AccessibilityState {
                role: accessibility_role(
                    widget.kind,
                    &widget.props.role,
                    &widget.props.element_tag,
                    accessible_name.as_deref(),
                    self.landmark_is_top_level(NodeHandle(widget.id), &widget.props.element_tag),
                ),
                label: accessible_name.map(Arc::<str>::from),
                value: (!widget.props.value.is_empty())
                    .then(|| Arc::<str>::from(widget.props.value.as_str())),
                disabled: widget.props.disabled,
                checked: matches!(
                    widget.kind,
                    crate::WidgetKind::Checkbox
                        | crate::WidgetKind::Switch
                        | crate::WidgetKind::Radio
                )
                .then_some(widget.props.toggled),
                selected: matches!(
                    widget.kind,
                    crate::WidgetKind::Chip
                        | crate::WidgetKind::ListItem
                        | crate::WidgetKind::SidebarRow
                        | crate::WidgetKind::TableRow
                        | crate::WidgetKind::InteractiveCard
                )
                .then_some(widget.props.active),
                multiline: matches!(widget.kind, crate::WidgetKind::Textarea),
                editable: matches!(
                    widget.kind,
                    crate::WidgetKind::Input
                        | crate::WidgetKind::NumberInput
                        | crate::WidgetKind::Textarea
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
                crate::WidgetKind::Input
                    | crate::WidgetKind::NumberInput
                    | crate::WidgetKind::Textarea
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
        PreparedSemanticSync {
            mutations,
            pending,
            component_owned_layout,
            projected,
            full_pass,
            revision: snapshot.revision,
        }
    }

    fn apply_semantic_styles(&mut self, prepared: PreparedSemanticSync) {
        let PreparedSemanticSync {
            mutations,
            pending,
            component_owned_layout,
            projected,
            full_pass,
            revision,
        } = prepared;
        if full_pass {
            self.component_owned_layout = component_owned_layout;
        } else {
            // Untouched widgets keep their ownership; dirty widgets re-decide.
            for id in &projected {
                self.component_owned_layout.remove(id);
            }
            self.component_owned_layout.extend(component_owned_layout);
        }
        self.commit_extra(mutations).ok();
        pending.apply(self.runtime.context_mut());
        self.adopt_runtime_allocated_ids();
        self.flush_runtime_systems();
        self.synced_semantic_revision = Some(revision);
    }

    /// Revision of the last fully or incrementally applied semantic snapshot.
    pub fn synced_semantic_revision(&self) -> Option<u64> {
        self.synced_semantic_revision
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
            let handle = self.materialize_fragment(frag, namespace, &mut created, true);
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
        mark_static: bool,
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
                if mark_static {
                    self.set_attribute(el, "data-static", "1");
                }
                created.push(el);
                for child in children {
                    let child_ns = match self.element_namespace(el) {
                        Some(n) => n,
                        None => ns,
                    };
                    let ch = self.materialize_fragment(child, child_ns, created, mark_static);
                    self.insert(ch, el, None);
                }
                el
            }
        }
    }

    /// Vue `v-html` / `innerHTML`: parse a fragment into live children.
    pub fn set_inner_html(&mut self, el: NodeHandle, html: &str) -> Vec<NodeHandle> {
        if !matches!(
            self.nodes.get(&el.0).map(|node| &node.data),
            Some(NodeData::Element { .. })
        ) {
            return Vec::new();
        }
        let children = self.children_of(el);
        for child in children {
            self.dispose_subtree(child);
        }
        self.set_attribute(el, "innerHTML", html);
        if html.is_empty() {
            return Vec::new();
        }
        let roots = parse_html_fragment(html);
        let mut created = Vec::new();
        let ns = self.element_namespace(el).unwrap_or(ElementNamespace::Html);
        for frag in roots {
            let handle = self.materialize_fragment(frag, ns, &mut created, false);
            self.insert(handle, el, None);
        }
        created
    }

    pub fn attributes(&self, el: NodeHandle) -> Vec<(String, String)> {
        match self.nodes.get(&el.0) {
            Some(Node {
                data: NodeData::Element { attrs, .. },
                ..
            }) => attrs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        }
    }

    pub fn set_gpu_slot(&mut self, el: NodeHandle, slot: &str) {
        self.set_attribute(el, "data-nana-gpu", slot);
    }

    /// First element carrying `name="value"`. Event dispatch for host-fed
    /// surfaces (`data-nana-video`) resolves targets this way; callers scan
    /// rarely, so no secondary index is maintained.
    pub(crate) fn element_with_attribute(&self, name: &str, value: &str) -> Option<NodeHandle> {
        self.nodes
            .iter()
            .filter(|(_, node)| matches!(node.data, NodeData::Element { .. }))
            .find_map(|(id, _)| {
                let handle = NodeHandle(*id);
                (self.get_attribute(handle, name).as_deref() == Some(value)).then_some(handle)
            })
    }

    /// CPU media ids still on the tree (video **and** audio) plus visual `video:{id}` slots.
    pub fn live_media_sets(&self) -> nana_ui_web_api::MediaLiveSets {
        let nodes: Vec<(String, Option<String>, Option<String>)> = self
            .nodes
            .keys()
            .copied()
            .map(|id| {
                let handle = NodeHandle(id);
                (
                    self.element_tag(handle).unwrap_or_default(),
                    self.get_attribute(handle, "data-nana-media"),
                    self.get_attribute(handle, "data-nana-video"),
                )
            })
            .collect();
        nana_ui_web_api::media_live_sets_from_tree(nodes.iter().map(|(tag, media_id, video_id)| {
            nana_ui_web_api::MediaTreeRef {
                tag,
                media_id: media_id.as_deref(),
                video_id: video_id.as_deref(),
            }
        }))
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
            host_texture_content(slot, revision).with_fit(self.surface_object_fit(el))
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
        if let Some(slot) = self
            .get_attribute(el, "data-nana-video")
            .as_deref()
            .and_then(video_host_texture_slot)
        {
            return Some(slot);
        }
        self.get_attribute(el, "data-nana-canvas")
            .or_else(|| self.get_attribute(el, "data-nana-image"))
            .as_deref()
            .and_then(canvas_host_texture_slot)
            .or_else(|| {
                self.get_attribute(el, "data-nana-video")
                    .as_deref()
                    .and_then(video_host_texture_slot)
            })
            .or_else(|| {
                self.svg_rasters
                    .contains_key(&el.0)
                    .then(|| format!("svg:{}", el.0))
            })
    }

    fn surface_object_fit(&self, el: NodeHandle) -> nana_ui_core::ContentFit {
        if let Ok(id) = StableNodeId::try_from(el)
            && let Some(fit) = self
                .runtime
                .node_style(id)
                .and_then(|style| style.layout.paint.object_fit)
        {
            return background_image_fit_to_content(fit);
        }
        if let Some(fit) = self
            .get_attribute(el, "object-fit")
            .as_deref()
            .and_then(nana_ui_core::ContentFit::from_object_fit)
        {
            return fit;
        }
        self.get_attribute(el, "style")
            .as_deref()
            .and_then(object_fit_from_css_text)
            .unwrap_or(nana_ui_core::ContentFit::Fill)
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
        if changed && is_host_texture_slot_attr(name) {
            self.index_host_texture_node(el);
            self.sync_surface_custom_render(el);
        } else if changed && is_host_texture_fit_attr(name) {
            self.sync_surface_custom_render(el);
        }
    }

    /// Paint-only CSS `transform` overlay (TransitionGroup FLIP).
    ///
    /// Writes Runtime `LayoutStyle.transform` so extract → UiScene →
    /// SceneWgpuPainter sees the affine. Never writes Runtime `LayoutBox`
    /// and never recascades selectors.
    pub fn set_paint_transform(&mut self, el: NodeHandle, css: &str) {
        if !self.nodes.contains_key(&el.0) {
            return;
        }
        let Ok(id) = StableNodeId::try_from(el) else {
            return;
        };
        let transform = crate::css_map::parse_inline_paint_transform(css);
        let mut style = self.runtime.node_style(id).cloned().unwrap_or_default();
        if style.layout.transform == transform {
            return;
        }
        let layout = Arc::make_mut(&mut style.layout);
        layout.transform = transform;
        self.pending.mutations.set_style(id, style);
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
        if removed && is_host_texture_slot_attr(name) {
            self.index_host_texture_node(el);
            self.sync_surface_custom_render(el);
        } else if removed && is_host_texture_fit_attr(name) {
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

    /// AccName for the semantic-sync path: author label, `aria-labelledby`
    /// (resolved against element `id`s in this document), or — for `section`/
    /// `form` landmarks only — name-from-content.
    fn semantic_accessible_name(&self, widget: &crate::SemanticWidget) -> Option<String> {
        if !widget.props.label.is_empty() {
            return Some(widget.props.label.clone());
        }
        let handle = NodeHandle(widget.id);
        if let Some(referred) = widget
            .props
            .attrs
            .get("aria-labelledby")
            .map(|ids| ids.trim())
            .filter(|ids| !ids.is_empty())
        {
            let names: Vec<String> = referred
                .split_whitespace()
                .filter_map(|id| self.element_with_id(id))
                .filter_map(|node| {
                    let attr_name = self.nodes.get(&node.0).and_then(|n| match &n.data {
                        NodeData::Element { attrs, .. } => attrs
                            .get("aria-label")
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                        _ => None,
                    });
                    attr_name.or_else(|| {
                        self.text_content(node)
                            .map(|text| text.trim().to_string())
                            .filter(|text| !text.is_empty())
                    })
                })
                .collect();
            if !names.is_empty() {
                return Some(names.join(" "));
            }
        }
        if matches!(
            widget
                .props
                .element_tag
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "section" | "form"
        ) {
            let text = self.text_content(handle)?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    }

    fn element_with_id(&self, id: &str) -> Option<NodeHandle> {
        self.nodes.iter().find_map(|(raw, node)| match &node.data {
            NodeData::Element { attrs, .. } if attrs.get("id").map(String::as_str) == Some(id) => {
                Some(NodeHandle(*raw))
            }
            _ => None,
        })
    }

    /// ARIA in HTML: `header`/`footer` are only banner/contentinfo when they
    /// are not descendants of `article`/`aside`/`main`/`nav`/`section`.
    fn landmark_is_top_level(&self, node: NodeHandle, tag: &str) -> bool {
        if !matches!(
            tag.trim().to_ascii_lowercase().as_str(),
            "header" | "footer"
        ) {
            return true;
        }
        let mut current = self.parent_element(node);
        while let Some(parent) = current {
            if matches!(
                self.element_tag(parent).as_deref(),
                Some("article" | "aside" | "main" | "nav" | "section")
            ) {
                return false;
            }
            current = self.parent_element(parent);
        }
        true
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
        if let std::collections::hash_map::Entry::Vacant(e) = self.nodes.entry(id) {
            e.insert(Node {
                data: NodeData::Element {
                    namespace: ElementNamespace::Html,
                    attrs: HashMap::from([(
                        crate::bridge::GENERATED_PSEUDO_ATTR.into(),
                        match pseudo {
                            crate::css_interactive::GeneratedPseudo::Before => "before",
                            crate::css_interactive::GeneratedPseudo::After => "after",
                            crate::css_interactive::GeneratedPseudo::Placeholder => "placeholder",
                        }
                        .into(),
                    )]),
                },
                scope_id: None,
            });
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
            self.svg_rasters.remove(&id);
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

struct CachedSvgRaster {
    markup_hash: u64,
    pixel_width: u32,
    pixel_height: u32,
    #[cfg_attr(not(any(test, feature = "hosted")), allow(dead_code))]
    raster: crate::svg_raster::RasterizedSvg,
    version: u64,
}

fn hash_svg_markup(markup: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    markup.hash(&mut hasher);
    hasher.finish()
}

fn parse_svg_intrinsic_size(
    width: Option<&str>,
    height: Option<&str>,
    view_box: Option<&str>,
) -> (u32, u32) {
    let parse_len = |raw: &str| {
        raw.trim()
            .trim_end_matches("px")
            .parse::<f32>()
            .ok()
            .filter(|value| *value > 0.0)
            .map(|value| value.round() as u32)
    };
    if let (Some(width), Some(height)) = (width.and_then(parse_len), height.and_then(parse_len)) {
        return (width.min(2048), height.min(2048));
    }
    if let Some(view_box) = view_box {
        let parts: Vec<f32> = view_box
            .split(|ch: char| ch.is_whitespace() || ch == ',')
            .filter_map(|part| part.parse().ok())
            .collect();
        if parts.len() == 4 && parts[2] > 0.0 && parts[3] > 0.0 {
            return (
                parts[2].round().max(1.0) as u32,
                parts[3].round().max(1.0) as u32,
            );
        }
    }
    (0, 0)
}

fn is_host_texture_slot_attr(name: &str) -> bool {
    name.eq_ignore_ascii_case("data-nana-gpu")
        || name.eq_ignore_ascii_case("data-nana-canvas")
        || name.eq_ignore_ascii_case("data-nana-image")
        || name.eq_ignore_ascii_case("data-nana-video")
}

fn is_host_texture_fit_attr(name: &str) -> bool {
    name.eq_ignore_ascii_case("style") || name.eq_ignore_ascii_case("object-fit")
}

fn background_image_fit_to_content(
    fit: nana_ui_core::BackgroundImageFit,
) -> nana_ui_core::ContentFit {
    use nana_ui_core::{BackgroundImageFit, ContentFit};
    match fit {
        BackgroundImageFit::Cover => ContentFit::Cover,
        BackgroundImageFit::Contain => ContentFit::Contain,
        BackgroundImageFit::Stretch | BackgroundImageFit::Length => ContentFit::Fill,
        BackgroundImageFit::Auto => ContentFit::None,
        BackgroundImageFit::ScaleDown => ContentFit::ScaleDown,
    }
}

fn object_fit_from_css_text(css: &str) -> Option<nana_ui_core::ContentFit> {
    let mut found = None;
    for decl in css.split(';') {
        let Some((name, value)) = decl.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("object-fit")
            && let Some(fit) = nana_ui_core::ContentFit::from_object_fit(value)
        {
            found = Some(fit);
        }
    }
    found
}

#[cfg(test)]
mod tests;
