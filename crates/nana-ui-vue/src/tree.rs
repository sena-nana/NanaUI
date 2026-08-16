//! Vue host adapter backed by the Runtime's authoritative retained tree.
//!
//! Keeps createElement / insert / patchProp / event flags for the JS custom
//! renderer and capability bridge. Identity, hierarchy, node kind, text,
//! focus, style, interaction, and layout live in `nana_ui_runtime::UiWorld`;
//! this module retains only Vue compatibility metadata.
//!
//! Vue mixed-tree layout writers are Style-Model [`crate::measure_layout`]
//! (pre-paint / first insert) and Iced `LayoutProbe` writeback after paint.
//! [`LayoutBoxStore`] is the JS paint projection. Those boxes are adapted into
//! `UiWorld` for Scene extraction and hit-test. `RuntimeLayoutEngine` is the
//! writer for `run_runtime` documents and is not run on Vue trees.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use nana_ui_runtime::{
    AccessibilityDelta, AccessibilityRole, AccessibilityState, AccessibilityUpdate,
    ActionMenu as RuntimeActionMenu, ActionMenuItem as RuntimeActionMenuItem,
    Button as RuntimeButton, Card as RuntimeCard, Checkbox as RuntimeCheckbox, ComponentView,
    ConfirmDialog as RuntimeConfirmDialog, ContextMenu as RuntimeContextMenu,
    ContextMenuItem as RuntimeContextMenuItem, CustomRenderNode, Dialog as RuntimeDialog,
    Drawer as RuntimeDrawer, EmptyState as RuntimeEmptyState, FormField as RuntimeFormField,
    IconButton as RuntimeIconButton, ImeComposition, InteractionState,
    InteractiveCard as RuntimeInteractiveCard, LabeledValue as RuntimeLabeledValue,
    LayoutBox as RuntimeLayoutBox, LevelMeter as RuntimeLevelMeter, ListItem as RuntimeListItem,
    ListItemSlots, ModalSurface, MutationQueue, NodeKind, NodeStyle, Popover as RuntimePopover,
    Progress as RuntimeProgress, QrCode as RuntimeQrCode, RangeField as RuntimeRangeField,
    SegmentedControl as RuntimeSegmentedControl, SegmentedOption as RuntimeSegmentedOption,
    Select as RuntimeSelect, SelectOption as RuntimeSelectOption, SelectionChrome,
    SettingsCard as RuntimeSettingsCard, SettingsRow as RuntimeSettingsRow,
    SidebarFrame as RuntimeSidebarFrame, SidebarRow as RuntimeSidebarRow,
    Skeleton as RuntimeSkeleton, Spinner as RuntimeSpinner, StableNodeId,
    StatusBadge as RuntimeStatusBadge, Switch as RuntimeSwitch, TextArea as RuntimeTextArea,
    TextContent, TextInput as RuntimeTextInput, TextInputState, Toast as RuntimeToast,
    Tooltip as RuntimeTooltip, UiWorld, ValidationMessage as RuntimeValidationMessage,
    ValueEmphasis, XYPad as RuntimeXYPad,
};
use nana_ui_scene::UiScene;

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

/// Layout box in logical CSS px (viewport / iced absolute coordinates).
///
/// Sources, in preference order for JS `getBoundingClientRect` / `layoutBox`:
/// 1. iced paint writeback ([`LayoutBoxStore`])
/// 2. Style-Model [`crate::measure_layout`] applied via [`NanaTreeDocument::apply_layout_boxes`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    pub handle: NodeHandle,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Shared iced → host layout writeback buffer.
///
/// Cleared at the start of each semantic `view` build; refilled when iced draws
/// each probed widget. `layoutBox` / [`get_layout_box`] read this first so menu
/// and popover anchors track real paint geometry (including scroll/chrome offsets
/// that Style-Model measure does not see).
#[derive(Debug, Default)]
pub struct LayoutBoxStore {
    boxes: Mutex<HashMap<u64, LayoutBox>>,
    transforms: Mutex<HashMap<u64, (LayoutBox, [f32; 6])>>,
}

impl LayoutBoxStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop prior-frame entries before rebuilding the iced element tree.
    pub fn begin_frame(&self) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.transforms.lock() {
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
        if let Ok(mut guard) = self.transforms.lock() {
            guard.insert(handle.0, (source, affine));
        }
    }

    pub fn contains_point(&self, handle: NodeHandle, x: f32, y: f32) -> bool {
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
        let transformed = self
            .transforms
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied());
        if let Some((mut source, mut affine)) = transformed {
            source.x += dx;
            source.y += dy;
            affine[4] += dx;
            affine[5] += dy;
            let box_ = transform_layout_box(source, affine);
            if let Ok(mut boxes) = self.boxes.lock() {
                boxes.insert(handle.0, box_);
            }
            if let Ok(mut transforms) = self.transforms.lock() {
                transforms.insert(handle.0, (source, affine));
            }
            Some(box_)
        } else {
            let mut box_ = self.get(handle)?;
            box_.x += dx;
            box_.y += dy;
            self.record(handle, box_.x, box_.y, box_.width, box_.height);
            Some(box_)
        }
    }

    pub fn get(&self, handle: NodeHandle) -> Option<LayoutBox> {
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

    /// Snapshot for [`NanaTreeDocument::apply_layout_boxes`].
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

/// Legacy store for standalone semantic-view helpers.
///
/// `VueHost` owns an isolated store and passes it to its probes and host ops;
/// hosted/multi-window code must not use this process-wide fallback.
pub fn shared_layout_box_store() -> Arc<LayoutBoxStore> {
    static STORE: OnceLock<Arc<LayoutBoxStore>> = OnceLock::new();
    Arc::clone(STORE.get_or_init(|| Arc::new(LayoutBoxStore::new())))
}

/// Prefer `store` writeback, else the document's pre-paint measure cache.
pub fn get_layout_box_from(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    handle: NodeHandle,
) -> Option<LayoutBox> {
    store.get(handle).or_else(|| doc.layout_box(handle))
}

/// Prefer iced writeback, else the document's pre-paint measure cache.
pub fn get_layout_box(doc: &NanaTreeDocument, handle: NodeHandle) -> Option<LayoutBox> {
    get_layout_box_from(&shared_layout_box_store(), doc, handle)
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

/// In-memory DOM-ish tree for Vue host ops (no CSS engine).
pub struct NanaTreeDocument {
    id: DocumentId,
    runtime: UiWorld,
    scene: UiScene,
    nodes: HashMap<u64, Node>,
    next_id: u64,
    html_root: NodeHandle,
    mount_root: NodeHandle,
    event_flags: HashSet<(u64, String)>,
    gpu_slots: HashMap<u64, String>,
    stylesheets: Vec<String>,
    theme: String,
    logical_width: f32,
    logical_height: f32,
    scale_factor: f32,
    synced_semantic_revision: Option<u64>,
    pending_accessibility_updated: BTreeMap<StableNodeId, nana_ui_runtime::AccessibilityNode>,
    pending_accessibility_removed: BTreeSet<StableNodeId>,
    pending_accessibility_generation: u64,
    accessibility_full_required: bool,
}

const MAX_PENDING_ACCESSIBILITY_CHANGES: usize = 4_096;

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
        let mut runtime = UiWorld::new();
        let runtime_document =
            nana_ui_runtime::DocumentId::try_from(id).expect("Vue document IDs are nonzero");
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
            scene: UiScene::new(),
            nodes,
            next_id: node_base + 3,
            html_root: NodeHandle(html_root),
            mount_root: NodeHandle(mount_root),
            event_flags: HashSet::new(),
            gpu_slots: HashMap::new(),
            stylesheets: Vec::new(),
            theme: "light".into(),
            logical_width,
            logical_height,
            scale_factor: scale,
            synced_semantic_revision: None,
            pending_accessibility_updated: BTreeMap::new(),
            pending_accessibility_removed: BTreeSet::new(),
            pending_accessibility_generation: 0,
            accessibility_full_required: false,
        };
        doc.reset_layout_roots();
        doc
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn runtime_generation(&self) -> u64 {
        self.runtime.generation()
    }

    pub fn scroll_offset(&self, node: NodeHandle) -> nana_ui_runtime::ScrollOffset {
        StableNodeId::try_from(node)
            .ok()
            .and_then(|id| self.runtime.scroll_offset(id))
            .unwrap_or_default()
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
        if self.runtime.scroll_offset(id) == Some(offset) {
            return false;
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(id, offset);
        self.runtime.commit(mutations).is_ok()
    }

    pub(crate) fn sync_scroll_viewport(
        &mut self,
        node: NodeHandle,
        offset: nana_ui_runtime::ScrollOffset,
        metrics: nana_ui_runtime::ScrollMetrics,
    ) -> Option<(nana_ui_runtime::ScrollOffset, nana_ui_runtime::ScrollOffset)> {
        let id = StableNodeId::try_from(node).ok()?;
        let previous = self.runtime.scroll_offset(id)?;
        if self.runtime.scroll_metrics(id) == Some(metrics)
            && self.runtime.clamp_scroll_offset(id, offset) == previous
        {
            return None;
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_metrics(id, Some(metrics));
        mutations.set_scroll_offset(id, offset);
        self.runtime.commit(mutations).ok()?;
        Some((previous, self.runtime.scroll_offset(id)?))
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
        let mut mutations = MutationQueue::new();
        mutations.set_text_input(id, Some(state));
        self.runtime.commit(mutations).is_ok()
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
        let mut mutations = MutationQueue::new();
        mutations.set_ime(id, composition);
        self.runtime.commit(mutations).is_ok()
    }

    pub fn scene(&self) -> &UiScene {
        &self.scene
    }

    pub fn accessibility_snapshot(&self) -> Vec<nana_ui_runtime::AccessibilityNode> {
        self.runtime.project_accessibility(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
        )
    }

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
            return;
        }
        let mut mutations = MutationQueue::new();
        if self.runtime.theme_mode() != snapshot.theme {
            mutations.set_theme(snapshot.theme);
        }
        for widget in &snapshot.widgets {
            let Some(id) = StableNodeId::new(widget.id).filter(|id| self.runtime.contains(*id))
            else {
                continue;
            };
            // Qualified components own their complete Runtime projection. Queuing
            // a generic style/interaction/accessibility state first only to
            // overwrite it in the same transaction doubles validation, dirty
            // propagation and commit work for large component trees.
            if !matches!(
                widget.kind,
                crate::WidgetKind::Input
                    | crate::WidgetKind::Textarea
                    | crate::WidgetKind::ContextMenu
                    | crate::WidgetKind::Select
            ) && matches!(
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
            ) && self.runtime.text_input(id).is_some()
            {
                // Queue before the new component projection: SetTextInput(None)
                // clears the old committed value, then Button/ListItem may publish
                // their own visible label later in the same transaction.
                mutations.set_text_input(id, None);
            }
            if project_migrating_component(widget, snapshot, id, &self.runtime, &mut mutations) {
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
                pointer_events: !widget.props.disabled && !widget.props.layout.hidden,
                focusable: !widget.props.disabled
                    && matches!(
                        widget.kind,
                        crate::WidgetKind::Button
                            | crate::WidgetKind::Chip
                            | crate::WidgetKind::Input
                            | crate::WidgetKind::Textarea
                            | crate::WidgetKind::Checkbox
                            | crate::WidgetKind::Switch
                            | crate::WidgetKind::Select
                            | crate::WidgetKind::Tabs
                            | crate::WidgetKind::Segmented
                            | crate::WidgetKind::Range
                    ),
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
        let _ = self.runtime.commit(mutations);
        self.flush_runtime_systems();
        self.synced_semantic_revision = Some(snapshot.revision);
    }

    pub(crate) fn apply_runtime_hierarchy(&self, snapshot: &mut crate::SemanticSnapshot) {
        snapshot.widgets.retain(|widget| {
            StableNodeId::new(widget.id).is_some_and(|id| self.runtime.contains(id))
        });
        let visible = snapshot
            .widgets
            .iter()
            .map(|widget| widget.id)
            .collect::<HashSet<_>>();
        for widget in &mut snapshot.widgets {
            let node = self
                .runtime
                .node(StableNodeId::new(widget.id).expect("retained widget ID is nonzero"))
                .expect("retained widget remains live");
            widget.parent = node
                .parent
                .map(StableNodeId::get)
                .filter(|parent| visible.contains(parent));
            widget.children = node
                .children
                .into_iter()
                .map(StableNodeId::get)
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
        let Ok(parent) = StableNodeId::try_from(parent) else {
            return Vec::new();
        };
        self.runtime
            .node(parent)
            .filter(|_| {
                matches!(
                    self.nodes.get(&parent.get()).map(|node| &node.data),
                    Some(NodeData::Element { .. })
                )
            })
            .map(|node| node.children.into_iter().map(NodeHandle::from).collect())
            .unwrap_or_default()
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
        let mut mutations = MutationQueue::new();
        mutations.create(
            StableNodeId::new(id).expect("allocated IDs are nonzero"),
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            NodeKind::Element {
                tag: tag.to_ascii_lowercase(),
            },
        );
        self.runtime
            .commit(mutations)
            .expect("allocated element is unique");
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
        let mut mutations = MutationQueue::new();
        let id = StableNodeId::new(id).expect("allocated IDs are nonzero");
        mutations.create(
            id,
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            NodeKind::Text,
        );
        mutations.set_text(id, TextContent { value: text.into() });
        self.runtime
            .commit(mutations)
            .expect("allocated text is unique");
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
        let mut mutations = MutationQueue::new();
        let id = StableNodeId::new(id).expect("allocated IDs are nonzero");
        mutations.create(
            id,
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            NodeKind::Comment,
        );
        mutations.set_text(id, TextContent { value: text.into() });
        self.runtime
            .commit(mutations)
            .expect("allocated comment is unique");
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

    pub fn gpu_slots(&self) -> &HashMap<u64, String> {
        &self.gpu_slots
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
        let mut mutations = MutationQueue::new();
        mutations.insert(
            StableNodeId::try_from(parent).expect("known parent is nonzero"),
            StableNodeId::try_from(child).expect("known child is nonzero"),
            anchor.and_then(|anchor| StableNodeId::try_from(anchor).ok()),
        );
        let _ = self.runtime.commit(mutations);
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
            let mut mutations = MutationQueue::new();
            mutations.set_text(
                StableNodeId::try_from(node).expect("known text is nonzero"),
                TextContent { value: text.into() },
            );
            let _ = self.runtime.commit(mutations);
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
        if changed && name.eq_ignore_ascii_case("data-nana-gpu") {
            self.gpu_slots.insert(el.0, value.to_string());
            let id = StableNodeId::try_from(el).expect("known element ID is nonzero");
            let content = CustomRenderNode {
                renderer: Arc::from("nana.host-texture"),
                resource: Arc::from(value),
                revision: 0,
            };
            if self.runtime.custom_render(id) != Some(&content) {
                let mut mutations = MutationQueue::new();
                mutations.set_custom_render(id, Some(content));
                self.runtime
                    .commit(mutations)
                    .expect("known GPU slot node remains valid");
            }
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
        if removed && name.eq_ignore_ascii_case("data-nana-gpu") {
            self.gpu_slots.remove(&el.0);
            let id = StableNodeId::try_from(el).expect("known element ID is nonzero");
            if self.runtime.custom_render(id).is_some() {
                let mut mutations = MutationQueue::new();
                mutations.set_custom_render(id, None);
                self.runtime
                    .commit(mutations)
                    .expect("known GPU slot node remains valid");
            }
        }
    }

    pub fn set_event_flag(&mut self, el: NodeHandle, event: &str, enabled: bool) {
        let name = normalize_event_name(event);
        if enabled {
            self.event_flags.insert((el.0, name));
        } else {
            self.event_flags.remove(&(el.0, name));
        }
    }

    pub fn has_event(&self, el: NodeHandle, event: &str) -> bool {
        self.event_flags
            .contains(&(el.0, normalize_event_name(event)))
    }

    pub fn parent_node(&self, node: NodeHandle) -> Option<NodeHandle> {
        self.runtime
            .node(StableNodeId::try_from(node).ok()?)?
            .parent
            .map(NodeHandle::from)
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
        match StableNodeId::try_from(node)
            .ok()
            .and_then(|id| self.runtime.node(id))
            .map(|node| node.kind)
        {
            Some(NodeKind::Element { .. }) => DomNodeKind::Element,
            Some(NodeKind::Text) => DomNodeKind::Text,
            Some(NodeKind::Comment) => DomNodeKind::Comment,
            Some(NodeKind::Document) => DomNodeKind::Document,
            None => DomNodeKind::Other,
        }
    }

    pub fn element_tag(&self, node: NodeHandle) -> Option<String> {
        match self.runtime.node(StableNodeId::try_from(node).ok()?)?.kind {
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
        self.runtime
            .commit(mutations)
            .expect("root layout is finite");
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

    /// Replace layout cache with measured / iced-written boxes.
    ///
    /// Prefer calling this with [`LayoutBoxStore::snapshot`] after iced draws;
    /// Style-Model `measure_layout` is the headless / pre-paint fallback.
    ///
    /// Always keeps html/body covering the viewport so hit-tests still have a
    /// root surface when the forest is sparse.
    pub fn apply_layout_boxes(&mut self, boxes: &[(NodeHandle, LayoutBox)]) {
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
            if let Ok(id) = StableNodeId::try_from(handle)
                && self.runtime.contains(id)
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
        let _ = self.runtime.commit(mutations);
        self.flush_runtime_systems();
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
        let gpu_slots: Vec<_> = self
            .gpu_slots
            .iter()
            .map(|(&id, s)| (NodeHandle(id), s.clone()))
            .collect();
        BoxSnapshot {
            boxes,
            texts,
            tags,
            event_targets: self.event_flags.clone(),
            gpu_slots,
        }
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
        if !self.runtime.contains(target) {
            return false;
        }
        let document =
            nana_ui_runtime::DocumentId::try_from(self.id).expect("Vue document IDs are nonzero");
        if self.runtime.pointer_capture(document, pointer_id) == Some(target) {
            return true;
        }
        let mut mutations = MutationQueue::new();
        mutations.capture_pointer(pointer_id, target);
        self.runtime.commit(mutations).is_ok()
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
        let mut mutations = MutationQueue::new();
        mutations.release_pointer(pointer_id, target);
        self.runtime.commit(mutations).is_ok()
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
        let mut mutations = MutationQueue::new();
        for (pointer_id, target) in captures {
            mutations.release_pointer(pointer_id, target);
        }
        self.runtime
            .commit(mutations)
            .expect("current pointer captures must release atomically");
    }

    pub fn take_pointer_capture_changes(&mut self) -> Vec<nana_ui_runtime::PointerCaptureChange> {
        self.runtime.take_pointer_capture_changes()
    }

    pub fn set_focus(&mut self, node: NodeHandle) {
        let Ok(node) = StableNodeId::try_from(node) else {
            return;
        };
        if !self.runtime.contains(node) {
            return;
        }
        let mut mutations = MutationQueue::new();
        mutations.set_interaction(
            node,
            nana_ui_runtime::InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        mutations.request_focus(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            Some(node),
        );
        let _ = self.runtime.commit(mutations);
    }

    pub fn clear_focus(&mut self) {
        let mut mutations = MutationQueue::new();
        mutations.request_focus(
            nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            None,
        );
        let _ = self.runtime.commit(mutations);
    }

    pub fn focused(&self) -> Option<NodeHandle> {
        self.runtime
            .focused(nana_ui_runtime::DocumentId::try_from(self.id).ok()?)
            .map(NodeHandle::from)
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
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(StableNodeId::try_from(root).expect("known node is nonzero"));
        if self.runtime.commit(mutations).is_err() {
            return;
        }
        for id in ids {
            self.nodes.remove(&id);
            self.gpu_slots.remove(&id);
            self.event_flags.retain(|(event_id, _)| *event_id != id);
        }
    }

    fn collect_preorder(&self, id: u64, out: &mut Vec<u64>) {
        out.push(id);
        for child in self.children_of(NodeHandle(id)) {
            self.collect_preorder(child.0, out);
        }
    }

    fn runtime_text(&self, node: NodeHandle) -> Option<String> {
        self.runtime
            .text(StableNodeId::try_from(node).ok()?)
            .map(str::to_owned)
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
        let work = self.runtime.take_system_work();
        self.runtime
            .resolve_styles(&work.style)
            .expect("scheduled style nodes remain live");
        #[cfg(feature = "iced-view")]
        self.runtime
            .shape_text(&work.text, &mut crate::IcedTextShaper)
            .expect("Iced shaping produces finite metrics");
        self.runtime.reconcile_focus(&work.focus_ime);
        self.record_accessibility_delta(self.runtime.project_accessibility_delta(&work));
        if !work.input_hit_test.is_empty() {
            self.runtime.rebuild_hit_test(
                nana_ui_runtime::DocumentId::try_from(self.id).expect("document ID is nonzero"),
            );
        }
        let extracted = self.runtime.extract_nodes(&work.render_extraction);
        self.scene.apply_delta(extracted, work.render_removals);
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

fn is_sidebar_frame_body(widget: &crate::SemanticWidget) -> bool {
    widget.props.attrs.get("data-slot").map(String::as_str) == Some("sidebar-body")
        || widget
            .props
            .class_names
            .iter()
            .any(|class| class.contains("nana-sidebar-frame__body"))
}

fn project_migrating_component(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) -> bool {
    match widget.kind {
        crate::WidgetKind::Button => {
            let icon = widget
                .children
                .iter()
                .filter_map(|child| snapshot.get(*child))
                .find(|child| child.kind == crate::WidgetKind::Icon)
                .and_then(|child| {
                    nana_ui_core::Icon::parse_name(child.props.display_label())
                        .or_else(|| nana_ui_core::Icon::parse_name(&child.props.value))
                })
                .or_else(|| nana_ui_core::Icon::parse_name(&widget.props.value));
            if let Some(icon) = icon {
                let label = if widget.props.hint.is_empty() {
                    widget.props.display_label()
                } else {
                    widget.props.hint.as_str()
                };
                let mut component = RuntimeIconButton::new(icon, Arc::<str>::from(label))
                    .kind(widget.props.button_kind)
                    .size(widget.props.size)
                    .selected(widget.props.active)
                    .disabled(widget.props.disabled);
                if !widget.props.hint.is_empty() {
                    component = component.tooltip(
                        Arc::<str>::from(widget.props.hint.as_str()),
                        nana_ui_core::TooltipConfig::default(),
                    );
                }
                component.project(id, world, mutations);
            } else {
                RuntimeButton::new(widget.props.display_label())
                    .layout(Arc::new(widget.props.layout.clone()))
                    .kind(widget.props.button_kind)
                    .size(widget.props.size)
                    .disabled(widget.props.disabled)
                    .loading(widget.props.loading)
                    .invalid(widget.props.invalid)
                    .project(id, world, mutations);
            }
            true
        }
        crate::WidgetKind::Input => {
            let mut state = world
                .text_input(id)
                .cloned()
                .unwrap_or_else(|| TextInputState::new(&widget.props.value));
            if state.value != widget.props.value {
                state.replace_value(&widget.props.value);
            }
            let placeholder = if widget.props.placeholder.is_empty() {
                widget.props.hint.as_str()
            } else {
                widget.props.placeholder.as_str()
            };
            let mut component = RuntimeTextInput::new("")
                .placeholder(Arc::<str>::from(placeholder))
                .layout(Arc::new(widget.props.layout.clone()))
                .size(widget.props.size)
                .disabled(widget.props.disabled)
                .loading(widget.props.loading)
                .read_only(widget.props.read_only)
                .secure(widget.props.secure)
                .invalid(widget.props.invalid);
            component.state = state;
            if !widget.props.label.is_empty() {
                component = component.label(Arc::<str>::from(widget.props.label.as_str()));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Textarea => {
            let mut state = world
                .text_input(id)
                .cloned()
                .unwrap_or_else(|| TextInputState::new(&widget.props.value));
            if state.value != widget.props.value {
                state.replace_value(&widget.props.value);
            }
            let placeholder = if widget.props.placeholder.is_empty() {
                widget.props.hint.as_str()
            } else {
                widget.props.placeholder.as_str()
            };
            let mut component = RuntimeTextArea::new("")
                .placeholder(Arc::<str>::from(placeholder))
                .disabled(widget.props.disabled)
                .invalid(widget.props.invalid);
            if let Some(nana_ui_core::LengthSpec::Px(height)) = widget.props.layout.height {
                component = component.height(height);
            }
            if !widget.props.label.is_empty() {
                component = component.label(Arc::<str>::from(widget.props.label.as_str()));
            }
            component.state = state;
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Select => {
            let placeholder = if widget.props.placeholder.is_empty() {
                widget.props.hint.as_str()
            } else {
                widget.props.placeholder.as_str()
            };
            if crate::widget_map::is_search_dropdown(&widget.props) {
                let value = (!widget.props.value.is_empty()).then_some(widget.props.value.as_str());
                let query = world
                    .text_input(id)
                    .map(|state| state.value.clone())
                    .unwrap_or_default();
                let mut component = nana_ui_runtime::SearchDropdown::new(value)
                    .options(widget.props.options.iter().map(|option| {
                        nana_ui_runtime::SearchDropdownOption::new(
                            option.value.as_str(),
                            option.label.as_str(),
                        )
                    }))
                    .query(query)
                    .size(widget.props.size)
                    .disabled(widget.props.disabled)
                    .loading(widget.props.loading)
                    .invalid(widget.props.invalid)
                    .opened(widget.props.active || widget.props.toggled);
                if !placeholder.is_empty() {
                    component = component.placeholder(Arc::<str>::from(placeholder));
                }
                component.project(id, world, mutations);
            } else if crate::widget_map::is_dropdown_field(&widget.props) {
                let multiple = widget.props.attrs.contains_key("multiple");
                let mut component = if multiple {
                    let values = if widget.props.value.is_empty() {
                        Vec::new()
                    } else {
                        widget
                            .props
                            .value
                            .split(',')
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .collect::<Vec<_>>()
                    };
                    nana_ui_runtime::Dropdown::multiple(values)
                } else {
                    nana_ui_runtime::Dropdown::single(
                        (!widget.props.value.is_empty()).then_some(widget.props.value.as_str()),
                    )
                };
                component = component
                    .options(widget.props.options.iter().map(|option| {
                        nana_ui_runtime::DropdownOption::new(
                            option.value.as_str(),
                            option.label.as_str(),
                        )
                        .disabled(option.disabled)
                    }))
                    .size(widget.props.size)
                    .disabled(widget.props.disabled)
                    .loading(widget.props.loading)
                    .invalid(widget.props.invalid)
                    .opened(widget.props.active || widget.props.toggled);
                if !placeholder.is_empty() {
                    component = component.placeholder(Arc::<str>::from(placeholder));
                }
                component.project(id, world, mutations);
            } else {
                let value = (!widget.props.value.is_empty()).then_some(widget.props.value.as_str());
                let mut component = RuntimeSelect::new(value)
                    .options(widget.props.options.iter().map(|option| {
                        RuntimeSelectOption::new(option.value.as_str(), option.label.as_str())
                            .disabled(option.disabled)
                    }))
                    .size(widget.props.size)
                    .disabled(widget.props.disabled)
                    .loading(widget.props.loading)
                    .invalid(widget.props.invalid)
                    .opened(widget.props.active || widget.props.toggled);
                if !placeholder.is_empty() {
                    component = component.placeholder(Arc::<str>::from(placeholder));
                }
                component.project(id, world, mutations);
            }
            true
        }
        crate::WidgetKind::Dialog => {
            if vue_confirm_dialog(&widget.props) {
                let message = if !widget.props.hint.is_empty() {
                    widget.props.hint.as_str()
                } else {
                    widget.props.value.as_str()
                };
                let mut component =
                    RuntimeConfirmDialog::new(widget.props.display_label(), message);
                component.danger = vue_confirm_danger(&widget.props);
                component.busy = widget.props.loading;
                component.project(id, world, mutations);
            } else {
                let mut component = RuntimeDialog::new(widget.props.display_label());
                if !widget.props.hint.is_empty() {
                    component = component.description(Arc::<str>::from(widget.props.hint.as_str()));
                }
                if let Some(body) = widget.children.first().copied().and_then(StableNodeId::new) {
                    component.slots.body = Some(body);
                }
                component.project(id, world, mutations);
            }
            true
        }
        crate::WidgetKind::Drawer => {
            let mut component = RuntimeDrawer::new(widget.props.display_label())
                .side(vue_drawer_side(&widget.props));
            if !widget.props.hint.is_empty() {
                component = component.description(Arc::<str>::from(widget.props.hint.as_str()));
            }
            if let Some(body) = widget.children.first().copied().and_then(StableNodeId::new) {
                component.slots_mut().body = Some(body);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Popover => {
            RuntimePopover::new()
                .trigger(widget.props.display_label())
                .open(widget.props.active || widget.props.toggled)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::ContextMenu => {
            let searchable = widget.props.options.len() >= 6
                || widget
                    .props
                    .class_names
                    .iter()
                    .any(|class| class.contains("search"));
            let query = if searchable {
                world
                    .text_input(id)
                    .map(|state| state.value.clone())
                    .or_else(|| {
                        crate::widget_map::attr_value(&widget.props, &["query", "data-query"])
                            .map(str::to_string)
                    })
                    .unwrap_or_default()
            } else {
                crate::widget_map::attr_value(&widget.props, &["query", "data-query"])
                    .unwrap_or("")
                    .to_string()
            };
            RuntimeContextMenu::new(widget.props.anchor_x, widget.props.anchor_y)
                .items(widget.props.options.iter().map(|option| {
                    RuntimeContextMenuItem::new(option.value.as_str(), option.label.as_str())
                        .disabled(option.disabled)
                }))
                .query(query)
                .searchable(searchable)
                .open(widget.props.active || widget.props.toggled)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::Toast => {
            let mut component = RuntimeToast::new(
                widget.props.display_label(),
                crate::widget_map::toast_tone_from_props(&widget.props),
            )
            .dismissible(crate::widget_map::toast_dismissible(&widget.props));
            if !widget.props.hint.is_empty() {
                component = component.description(Arc::<str>::from(widget.props.hint.as_str()));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Tooltip => {
            let label = if !widget.props.display_label().is_empty() {
                widget.props.display_label()
            } else {
                widget.props.hint.as_str()
            };
            RuntimeTooltip::new(label).project(id, world, mutations);
            if world.standard_visual(id).is_some() {
                mutations.set_standard_visual(id, None);
            }
            true
        }
        crate::WidgetKind::ActionMenu => {
            RuntimeActionMenu::new()
                .trigger(widget.props.display_label())
                .open(widget.props.active || widget.props.toggled)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::ActionMenuItem => {
            let mut component = RuntimeActionMenuItem::new(widget.props.display_label())
                .active(widget.props.active)
                .danger(crate::widget_map::action_menu_item_danger(&widget.props))
                .disabled(widget.props.disabled)
                .size(widget.props.size);
            if !widget.props.hint.is_empty() {
                component = component.hint(Arc::<str>::from(widget.props.hint.as_str()));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::XYPad => {
            let ((x_min, x_max), (y_min, y_max)) = crate::widget_map::xy_pad_ranges(&widget.props);
            let mut component = RuntimeXYPad::new(crate::widget_map::xy_pad_value(&widget.props))
                .x_range(x_min, x_max)
                .y_range(y_min, y_max)
                .size(widget.props.size)
                .disabled(widget.props.disabled)
                .loading(widget.props.loading)
                .invalid(widget.props.invalid);
            if widget.props.step.is_finite() && widget.props.step > 0.0 {
                component = component.step(widget.props.step);
            }
            let label = widget.props.display_label();
            if !label.is_empty() {
                component = component.label(Arc::<str>::from(label));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::QrCode => {
            let label = if widget.props.display_label().is_empty() {
                "QR code"
            } else {
                widget.props.display_label()
            };
            let size = match widget.props.layout.width {
                Some(nana_ui_core::LengthSpec::Px(px)) if px.is_finite() && px > 0.0 => px,
                _ => RuntimeQrCode::DEFAULT_SIZE,
            };
            if let Some((modules, width)) = qr_modules_from_props(&widget.props)
                && let Ok(component) = RuntimeQrCode::from_modules(modules, width, size)
            {
                component
                    .label(Arc::<str>::from(label))
                    .project(id, world, mutations);
                return true;
            }
            let payload =
                crate::widget_map::attr_value(&widget.props, &["payload", "data-payload", "value"])
                    .unwrap_or(widget.props.value.as_str());
            if !payload.is_empty()
                && let Ok(component) = RuntimeQrCode::encode(payload, size)
            {
                component
                    .label(Arc::<str>::from(label))
                    .project(id, world, mutations);
                return true;
            }
            RuntimeLabeledValue::new(label, payload).project(id, world, mutations);
            true
        }
        crate::WidgetKind::Checkbox => {
            RuntimeCheckbox::new(widget.props.display_label(), widget.props.toggled)
                .disabled(widget.props.disabled || widget.props.loading)
                .invalid(widget.props.invalid)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::Switch => {
            let mut component =
                RuntimeSwitch::new(widget.props.display_label(), widget.props.toggled)
                    .control_position(widget.props.control_position)
                    .size(widget.props.size)
                    .disabled(widget.props.disabled)
                    .loading(widget.props.loading)
                    .invalid(widget.props.invalid);
            if !widget.props.hint.is_empty() {
                component = component.hint(Arc::<str>::from(widget.props.hint.as_str()));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Card => {
            let mut component = RuntimeCard::new()
                .kind(widget.props.card_kind)
                .loading(widget.props.loading);
            if !widget.props.label.is_empty() {
                component = component.title(Arc::<str>::from(widget.props.label.as_str()));
            }
            if let Some(nana_ui_core::LengthSpec::Px(padding)) = widget.props.layout.padding_left {
                component = component.padding(padding);
            }
            if let Some(nana_ui_core::LengthSpec::Px(height)) = widget.props.layout.height {
                component = component.height(height);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::ListItem => {
            let slot = |name: &str| {
                widget.children.iter().find_map(|child| {
                    let child = snapshot.get(*child)?;
                    (child.props.attrs.get("data-slot").map(String::as_str) == Some(name))
                        .then(|| StableNodeId::new(child.id))
                        .flatten()
                })
            };
            let leading = slot("leading").or_else(|| slot("list-item-leading"));
            let trailing = slot("trailing").or_else(|| slot("list-item-trailing"));
            let content = slot("content")
                .or_else(|| slot("list-item-content"))
                .or_else(|| {
                    widget.children.iter().find_map(|child| {
                        let id = StableNodeId::new(*child)?;
                        (Some(id) != leading && Some(id) != trailing).then_some(id)
                    })
                });
            RuntimeListItem::new(widget.props.display_label())
                .slots(ListItemSlots {
                    leading,
                    content,
                    trailing,
                })
                .gap(widget.props.layout.gap_or(8.0))
                .size(widget.props.size)
                .auto_height(widget.props.auto_height)
                .selected(widget.props.active)
                .disabled(widget.props.disabled)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::Range => {
            let minimum = f64::from(widget.props.min);
            let supplied_maximum = f64::from(widget.props.max);
            let supplied_step = f64::from(widget.props.step);
            let malformed = !minimum.is_finite()
                || !supplied_maximum.is_finite()
                || supplied_maximum <= minimum
                || !supplied_step.is_finite()
                || supplied_step <= 0.0;
            let minimum = if minimum.is_finite() { minimum } else { 0.0 };
            let maximum = if supplied_maximum.is_finite() && supplied_maximum > minimum {
                supplied_maximum
            } else {
                minimum + 1.0
            };
            let step = if supplied_step.is_finite() && supplied_step > 0.0 {
                supplied_step
            } else {
                1.0
            };
            let value = f64::from(widget.props.number).clamp(minimum, maximum);
            let mut component = RuntimeRangeField::new(value, minimum, maximum, step)
                .expect("normalized Vue range contract is valid")
                .disabled(widget.props.disabled)
                .invalid(widget.props.invalid || malformed);
            if !widget.props.label.is_empty() {
                component = component.label(Arc::<str>::from(widget.props.label.as_str()));
            }
            if !widget.props.unit.is_empty() {
                component = component.unit(Arc::<str>::from(widget.props.unit.as_str()));
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::StatusBadge => {
            RuntimeStatusBadge::new(
                widget.props.display_label(),
                crate::widget_map::status_tone_from_props(&widget.props),
            )
            .compact(crate::widget_map::class_has_compact(&widget.props))
            .project(id, world, mutations);
            true
        }
        crate::WidgetKind::ValidationMessage => {
            let message = if !widget.props.hint.is_empty() {
                widget.props.hint.as_str()
            } else {
                widget.props.display_label()
            };
            RuntimeValidationMessage::new(
                message,
                crate::widget_map::validation_intent_from_props(&widget.props),
            )
            .project(id, world, mutations);
            true
        }
        crate::WidgetKind::EmptyState => {
            let mut component = RuntimeEmptyState::new(widget.props.display_label());
            if !widget.props.hint.is_empty() {
                component = component.message(Arc::<str>::from(widget.props.hint.as_str()));
            }
            if let Some(icon) = empty_state_icon(widget, snapshot) {
                component = component.icon(icon);
            }
            component = component.compact(crate::widget_map::class_has_compact(&widget.props));
            if let Some(action) = crate::widget_map::first_button_child_id(snapshot, widget)
                .and_then(StableNodeId::new)
            {
                component = component.action_child(action);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::LabeledValue => {
            let label = crate::widget_map::labeled_value_caption(snapshot, widget);
            let emphasis = if widget.props.muted {
                ValueEmphasis::Muted
            } else {
                ValueEmphasis::Strong
            };
            let mut component = RuntimeLabeledValue::new(label, widget.props.value.as_str())
                .emphasis(emphasis)
                .compact(crate::widget_map::class_has_compact(&widget.props));
            if let Some(action) = crate::widget_map::first_button_child_id(snapshot, widget)
                .and_then(StableNodeId::new)
            {
                component = component.action_child(action);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Segmented | crate::WidgetKind::Tabs => {
            let mut control = if widget.kind == crate::WidgetKind::Tabs {
                RuntimeSegmentedControl::tabs()
                    .size(widget.props.size)
                    .fill(widget.props.fill)
            } else {
                RuntimeSegmentedControl::new().size(widget.props.size)
            };
            if !widget.props.label.is_empty() {
                control = control.label(Arc::<str>::from(widget.props.label.as_str()));
            }
            let chrome = control.chrome_value();
            control.project(id, world, mutations);
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
                    world,
                    mutations,
                );
            }
            true
        }
        crate::WidgetKind::Progress => {
            let mut component = RuntimeProgress::new(
                f64::from(widget.props.progress),
                f64::from(widget.props.progress_max.max(1.0)),
            );
            let label = widget.props.display_label();
            if !label.is_empty() {
                component = component.label(Arc::<str>::from(label));
            }
            component =
                component.cancellable(crate::widget_map::progress_cancellable(&widget.props));
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Spinner => {
            RuntimeSpinner::new(widget.props.display_label()).project(id, world, mutations);
            true
        }
        crate::WidgetKind::FormField => {
            let mut component =
                RuntimeFormField::new(widget.props.display_label()).size(widget.props.size);
            let (hint, error) = crate::widget_map::form_field_support(&widget.props);
            if let Some(hint) = hint {
                component = component.hint(Arc::<str>::from(hint));
            }
            if let Some(error) = error {
                component = component.error(Arc::<str>::from(error));
            }
            if let Some(control) = crate::widget_map::form_field_control_child_id(snapshot, widget)
                .and_then(StableNodeId::new)
            {
                component = component.control_child(control);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::InteractiveCard => {
            RuntimeInteractiveCard::new()
                .selected(widget.props.active)
                .disabled(widget.props.disabled)
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::Column | crate::WidgetKind::Box if is_sidebar_frame_body(widget) => {
            RuntimeSidebarFrame::scroll_body(widget.props.layout.clone())
                .project(id, world, mutations);
            true
        }
        crate::WidgetKind::SidebarFrame => {
            let (top, body, footer) = crate::widget_map::sidebar_frame_slots(snapshot, widget);
            let mut component = RuntimeSidebarFrame::new().gap(widget.props.layout.gap_or(14.0));
            if let Some(top) = top.and_then(StableNodeId::new) {
                component = component.top(top);
            }
            if let Some(body) = body.and_then(StableNodeId::new) {
                component = component.body(body);
            }
            if let Some(footer) = footer.and_then(StableNodeId::new) {
                component = component.footer(footer);
            }
            component.style.layout = Arc::new(widget.props.layout.clone());
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::SidebarRow => {
            let slot = |name: &str| {
                widget.children.iter().find_map(|child| {
                    let child = snapshot.get(*child)?;
                    (child.props.attrs.get("data-slot").map(String::as_str) == Some(name))
                        .then(|| StableNodeId::new(child.id))
                        .flatten()
                })
            };
            let leading = slot("leading").or_else(|| slot("list-item-leading"));
            let trailing = slot("trailing").or_else(|| slot("list-item-trailing"));
            let content = slot("content")
                .or_else(|| slot("list-item-content"))
                .or_else(|| {
                    widget.children.iter().find_map(|child| {
                        let child_id = StableNodeId::new(*child)?;
                        (Some(child_id) != leading && Some(child_id) != trailing)
                            .then_some(child_id)
                    })
                });
            let mut component = RuntimeSidebarRow::new(widget.props.display_label())
                .slots(ListItemSlots {
                    leading,
                    content,
                    trailing,
                })
                .gap(widget.props.layout.gap_or(6.0))
                .size(widget.props.size)
                .state(crate::widget_map::sidebar_row_state(&widget.props))
                .tone(crate::widget_map::sidebar_row_tone(&widget.props))
                .depth(crate::widget_map::sidebar_row_depth(&widget.props));
            if let Some(expanded) = crate::widget_map::attr_value(
                &widget.props,
                &["expanded", "data-expanded", "disclosure"],
            )
            .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None,
            }) {
                component = component.disclosure(expanded);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::SettingsRow => {
            let (stacked, divided, loose, first, last) =
                crate::widget_map::settings_row_flags(&widget.props);
            let mut component = RuntimeSettingsRow::new(widget.props.display_label())
                .stacked(stacked)
                .divided(divided)
                .loose(loose);
            if !widget.props.hint.is_empty() {
                component = component.hint(Arc::<str>::from(widget.props.hint.as_str()));
            }
            if first {
                component = component.first_in_group();
            }
            if last {
                component = component.last_in_group();
            }
            if let Some(control) =
                crate::widget_map::settings_row_control_child_id(snapshot, widget)
                    .and_then(StableNodeId::new)
            {
                component = component.control_child(control);
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::SettingsCard => {
            let mut component = RuntimeSettingsCard::new(widget.props.display_label());
            let has_title_child = widget.children.iter().any(|child| {
                snapshot.get(*child).is_some_and(|child| {
                    child.props.class_names.iter().any(|class| {
                        class.contains("nana-settings-card__title")
                            || class.contains("settings-card__title")
                    })
                })
            });
            if has_title_child {
                component.title = Arc::from("");
            }
            component.project(id, world, mutations);
            true
        }
        crate::WidgetKind::Skeleton => {
            let width = widget
                .props
                .layout
                .width
                .unwrap_or(nana_ui_core::LengthSpec::Fill);
            let height = match widget.props.layout.height {
                Some(nana_ui_core::LengthSpec::Px(h)) if h.is_finite() && h > 0.0 => h,
                _ => 16.0,
            };
            RuntimeSkeleton::new(width, height).project(id, world, mutations);
            true
        }
        crate::WidgetKind::LevelMeter => {
            let mut component =
                RuntimeLevelMeter::new(crate::widget_map::level_meter_value(&widget.props))
                    .tone(crate::widget_map::status_tone_from_props(&widget.props));
            if let Some(nana_ui_core::LengthSpec::Px(height)) = widget.props.layout.height
                && height.is_finite()
                && height > 0.0
            {
                component = component.height(height);
            }
            component.project(id, world, mutations);
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
                )
            ) {
                mutations.set_standard_visual(id, None);
            }
            false
        }
    }
}

fn qr_modules_from_props(props: &crate::WidgetProps) -> Option<(Arc<[bool]>, usize)> {
    let raw = crate::widget_map::attr_value(props, &["modules", "data-modules"])?;
    let width_hint = crate::widget_map::attr_value(
        props,
        &["module-width", "modules-width", "data-module-width"],
    )
    .and_then(|raw| raw.trim().parse().ok());
    parse_qr_module_matrix(raw, width_hint)
}

fn parse_qr_module_matrix(raw: &str, width_hint: Option<usize>) -> Option<(Arc<[bool]>, usize)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cells: Vec<bool> = if trimmed.chars().all(|c| c == '0' || c == '1') {
        trimmed.chars().map(|c| c == '1').collect()
    } else {
        trimmed
            .split(|c: char| matches!(c, '[' | ']' | ',' | ';' | ' ' | '\n' | '\t' | '\r'))
            .filter(|part| !part.is_empty())
            .map(|part| match part {
                "1" | "true" | "TRUE" => Some(true),
                "0" | "false" | "FALSE" => Some(false),
                _ => None,
            })
            .collect::<Option<_>>()?
    };
    if cells.is_empty() {
        return None;
    }
    let width = width_hint.or_else(|| {
        let root = (cells.len() as f64).sqrt() as usize;
        (root > 0 && root.saturating_mul(root) == cells.len()).then_some(root)
    })?;
    if width == 0 || width.checked_mul(width) != Some(cells.len()) {
        return None;
    }
    Some((Arc::<[bool]>::from(cells), width))
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

fn vue_drawer_side(props: &crate::WidgetProps) -> nana_ui_core::DrawerSide {
    match props.side.to_ascii_lowercase().as_str() {
        "left" | "start" => nana_ui_core::DrawerSide::Left,
        _ => nana_ui_core::DrawerSide::Right,
    }
}

fn empty_state_icon(
    widget: &crate::SemanticWidget,
    snapshot: &crate::SemanticSnapshot,
) -> Option<nana_ui_core::Icon> {
    widget
        .children
        .iter()
        .filter_map(|child| snapshot.get(*child))
        .find(|child| child.kind == crate::WidgetKind::Icon)
        .and_then(|child| {
            nana_ui_core::Icon::parse_name(child.props.display_label())
                .or_else(|| nana_ui_core::Icon::parse_name(&child.props.value))
        })
        .or_else(|| nana_ui_core::Icon::parse_name(&widget.props.value))
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
        crate::WidgetKind::Select => AccessibilityRole::ComboBox,
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
        _ => AccessibilityRole::Generic,
    }
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
        doc.apply_layout_boxes(&[(
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

        doc.apply_layout_boxes(&[(
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
            Some(nana_ui_runtime::StandardVisual::Checkbox { checked: true })
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
    fn get_layout_box_prefers_iced_store_over_document() {
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
        // iced chrome (e.g. scrollable padding) shifts the painted box.
        store.record(child, 16.0, 16.0, 80.0, 24.0);
        let box_ = get_layout_box_from(&store, &doc, child).expect("iced box");
        assert_eq!(
            (box_.x, box_.y, box_.width, box_.height),
            (16.0, 16.0, 80.0, 24.0),
            "menu anchors must follow iced paint, not pre-paint measure"
        );
        store.begin_frame();
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
}
