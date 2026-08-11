//! Lightweight Vue host tree — no Blitz / full CSS engine.
//!
//! Keeps createElement / insert / patchProp / event flags for the JS custom
//! renderer and capability bridge. Layout boxes are synthetic (stack order,
//! optionally sized from inline `style` / class intent) so headless click
//! probes still work. Visible UI draws via [`crate::MessageBridge`] → iced-view.
//!
//! After iced paints, [`LayoutBoxStore`] holds authoritative viewport boxes
//! (written by iced-view `LayoutProbe`); [`get_layout_box`] / `layoutBox` prefer
//! those over the synthetic / measure cache so menu anchors match drawing.
//!
//! ## 诊断轨 / 禁止第二套解析
//!
//! [`NanaTreeDocument::resolve_now`] builds **diagnostic / hit-test** placeholders
//! via [`crate::style::StyleIntent`]. Inline declarations project from
//! [`crate::css_map::LayoutStyle`] — **do not** add a second CSS parse here.
//! Product geometry SoT remains iced probe → measure fallback → this synthetic
//! stack; do not force-merge the three tracks.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// Opaque handle returned to the Vue custom renderer (JSON-safe number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeHandle(pub u64);

impl NodeHandle {
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Stable document id for diagnostics (not a Blitz id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

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
/// 3. Synthetic stack placeholder from [`NanaTreeDocument::resolve_now`]
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

/// Process-wide store shared by iced-view probes and `layoutBox` host ops.
pub fn shared_layout_box_store() -> Arc<LayoutBoxStore> {
    static STORE: OnceLock<Arc<LayoutBoxStore>> = OnceLock::new();
    Arc::clone(STORE.get_or_init(|| Arc::new(LayoutBoxStore::new())))
}

/// Prefer `store` writeback, else document cache (measure / synthetic).
pub fn get_layout_box_from(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    handle: NodeHandle,
) -> Option<LayoutBox> {
    store.get(handle).or_else(|| doc.layout_box(handle))
}

/// Prefer iced writeback, else document cache (measure / synthetic).
pub fn get_layout_box(doc: &NanaTreeDocument, handle: NodeHandle) -> Option<LayoutBox> {
    get_layout_box_from(&shared_layout_box_store(), doc, handle)
}

/// Compact dump used by headless probes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSnapshot {
    pub boxes: Vec<LayoutBox>,
    pub texts: Vec<(NodeHandle, String)>,
    pub tags: Vec<(NodeHandle, String)>,
    pub backgrounds: Vec<(NodeHandle, [f32; 4])>,
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
        tag: String,
        namespace: ElementNamespace,
        attrs: HashMap<String, String>,
        children: Vec<u64>,
    },
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone)]
struct Node {
    parent: Option<u64>,
    data: NodeData,
    scope_id: Option<String>,
}

/// In-memory DOM-ish tree for Vue host ops (no CSS engine).
#[derive(Debug)]
pub struct NanaTreeDocument {
    id: DocumentId,
    nodes: HashMap<u64, Node>,
    next_id: u64,
    html_root: NodeHandle,
    mount_root: NodeHandle,
    event_flags: HashSet<(u64, String)>,
    gpu_slots: HashMap<u64, String>,
    stylesheets: Vec<String>,
    theme: String,
    focused: Option<NodeHandle>,
    logical_width: f32,
    logical_height: f32,
    scale_factor: f32,
    /// Synthetic layout cache (filled by [`resolve_now`]).
    layout: HashMap<u64, LayoutBox>,
}

// SAFETY: host ops run on the JS engine thread only (same contract as before).
unsafe impl Send for NanaTreeDocument {}
unsafe impl Sync for NanaTreeDocument {}

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
        // 1 = html, 2 = body (mount)
        nodes.insert(
            1,
            Node {
                parent: None,
                data: NodeData::Element {
                    tag: "html".into(),
                    namespace: ElementNamespace::Html,
                    attrs: HashMap::new(),
                    children: vec![2],
                },
                scope_id: None,
            },
        );
        nodes.insert(
            2,
            Node {
                parent: Some(1),
                data: NodeData::Element {
                    tag: "body".into(),
                    namespace: ElementNamespace::Html,
                    attrs: HashMap::new(),
                    children: Vec::new(),
                },
                scope_id: None,
            },
        );
        let mut doc = Self {
            id,
            nodes,
            next_id: 3,
            html_root: NodeHandle(1),
            mount_root: NodeHandle(2),
            event_flags: HashSet::new(),
            gpu_slots: HashMap::new(),
            stylesheets: Vec::new(),
            theme: "light".into(),
            focused: None,
            logical_width,
            logical_height,
            scale_factor: scale,
            layout: HashMap::new(),
        };
        doc.resolve_now();
        doc
    }

    pub fn id(&self) -> DocumentId {
        self.id
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
        match self.nodes.get(&parent.0) {
            Some(Node {
                data: NodeData::Element { children, .. },
                ..
            }) => children.iter().copied().map(NodeHandle).collect(),
            _ => Vec::new(),
        }
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
        self.resolve_now();
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
                parent: None,
                data: NodeData::Element {
                    tag: tag.to_ascii_lowercase(),
                    namespace,
                    attrs,
                    children: Vec::new(),
                },
                scope_id: None,
            },
        );
        NodeHandle(id)
    }

    pub fn create_text(&mut self, text: &str) -> NodeHandle {
        let id = self.alloc();
        self.nodes.insert(
            id,
            Node {
                parent: None,
                data: NodeData::Text(text.to_string()),
                scope_id: None,
            },
        );
        NodeHandle(id)
    }

    pub fn create_comment(&mut self, text: &str) -> NodeHandle {
        let id = self.alloc();
        self.nodes.insert(
            id,
            Node {
                parent: None,
                data: NodeData::Comment(text.to_string()),
                scope_id: None,
            },
        );
        NodeHandle(id)
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
            NodeData::Element {
                tag,
                namespace,
                attrs,
                children,
            } => {
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
                    for child in children {
                        let child_copy = self.clone_node_into(NodeHandle(child), true, pairs);
                        self.insert(child_copy, copy, None);
                    }
                }
                copy
            }
            NodeData::Text(t) => self.create_text(&t),
            NodeData::Comment(t) => self.create_comment(&t),
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
        self.gpu_slots.insert(el.0, slot.to_string());
        self.set_attribute(el, "data-nana-gpu", slot);
    }

    pub fn gpu_slots(&self) -> &HashMap<u64, String> {
        &self.gpu_slots
    }

    pub fn insert(&mut self, child: NodeHandle, parent: NodeHandle, anchor: Option<NodeHandle>) {
        self.detach(child);
        let Some(Node {
            data: NodeData::Element { children, .. },
            ..
        }) = self.nodes.get_mut(&parent.0)
        else {
            return;
        };
        let idx = anchor
            .and_then(|a| children.iter().position(|&c| c == a.0))
            .unwrap_or(children.len());
        children.insert(idx, child.0);
        if let Some(node) = self.nodes.get_mut(&child.0) {
            node.parent = Some(parent.0);
        }
    }

    pub fn remove(&mut self, child: NodeHandle) {
        // Teleport / v-if unmount: detach and drop the subtree so mount-root
        // open/close cycles do not accumulate orphan nodes (Overlay 不泄漏).
        self.dispose_subtree(child);
    }

    pub fn set_text(&mut self, node: NodeHandle, text: &str) {
        if let Some(Node {
            data: NodeData::Text(t),
            ..
        }) = self.nodes.get_mut(&node.0)
        {
            *t = text.to_string();
        }
    }

    pub fn set_element_text(&mut self, el: NodeHandle, text: &str) {
        let children = match self.nodes.get(&el.0) {
            Some(Node {
                data: NodeData::Element { children, .. },
                ..
            }) => children.clone(),
            _ => return,
        };
        for c in children {
            self.detach(NodeHandle(c));
            self.nodes.remove(&c);
        }
        let text_node = self.create_text(text);
        self.insert(text_node, el, None);
    }

    pub fn set_attribute(&mut self, el: NodeHandle, name: &str, value: &str) {
        if let Some(Node {
            data: NodeData::Element { attrs, .. },
            ..
        }) = self.nodes.get_mut(&el.0)
        {
            attrs.insert(name.to_string(), value.to_string());
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
        if let Some(Node {
            data: NodeData::Element { attrs, .. },
            ..
        }) = self.nodes.get_mut(&el.0)
        {
            attrs.remove(name);
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
        self.nodes
            .get(&node.0)
            .and_then(|n| n.parent.map(NodeHandle))
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
        let Node {
            data: NodeData::Element { children, .. },
            ..
        } = self.nodes.get(&parent.0)?
        else {
            return None;
        };
        let idx = children.iter().position(|&c| c == node.0)?;
        children.get(idx + 1).copied().map(NodeHandle)
    }

    pub fn previous_sibling(&self, node: NodeHandle) -> Option<NodeHandle> {
        let parent = self.parent_node(node)?;
        let Node {
            data: NodeData::Element { children, .. },
            ..
        } = self.nodes.get(&parent.0)?
        else {
            return None;
        };
        let idx = children.iter().position(|&c| c == node.0)?;
        if idx == 0 {
            None
        } else {
            children.get(idx - 1).copied().map(NodeHandle)
        }
    }

    /// DOM `Node.firstChild`.
    pub fn first_child(&self, parent: NodeHandle) -> Option<NodeHandle> {
        match self.nodes.get(&parent.0).map(|n| &n.data) {
            Some(NodeData::Element { children, .. }) => children.first().copied().map(NodeHandle),
            _ => None,
        }
    }

    pub fn last_child(&self, parent: NodeHandle) -> Option<NodeHandle> {
        match self.nodes.get(&parent.0).map(|n| &n.data) {
            Some(NodeData::Element { children, .. }) => children.last().copied().map(NodeHandle),
            _ => None,
        }
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
        let mut stack = vec![self.html_root.0];
        while let Some(id) = stack.pop() {
            if let Some(Node {
                data:
                    NodeData::Element {
                        tag,
                        attrs,
                        children,
                        ..
                    },
                ..
            }) = self.nodes.get(&id)
            {
                if selector_list_matches(sel, tag, attrs) {
                    out.push(NodeHandle(id));
                }
                for &c in children.iter().rev() {
                    stack.push(c);
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
                data: NodeData::Element { tag, attrs, .. },
                ..
            }) = self.nodes.get(&h.0)
            {
                if selector_list_matches(sel, tag, attrs) {
                    return Some(h);
                }
            }
            cur = self.parent_node(h);
        }
        None
    }

    pub fn node_kind(&self, node: NodeHandle) -> DomNodeKind {
        match self.nodes.get(&node.0).map(|n| &n.data) {
            Some(NodeData::Element { .. }) => DomNodeKind::Element,
            Some(NodeData::Text(_)) => DomNodeKind::Text,
            Some(NodeData::Comment(_)) => DomNodeKind::Comment,
            None => DomNodeKind::Other,
        }
    }

    pub fn element_tag(&self, node: NodeHandle) -> Option<String> {
        match self.nodes.get(&node.0).map(|n| &n.data) {
            Some(NodeData::Element { tag, .. }) => Some(tag.clone()),
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
        match self.nodes.get(&node.0).map(|n| &n.data) {
            Some(NodeData::Text(t)) => Some(t.clone()),
            Some(NodeData::Element { children, .. }) => {
                let mut out = String::new();
                for &c in children {
                    if let Some(t) = self.text_content(NodeHandle(c)) {
                        out.push_str(&t);
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Rebuilds synthetic stack-order layout (not CSS cascade).
    ///
    /// Direct children of the mount root are stacked vertically. Deeper
    /// descendants nest inside their parent's box (padding-aware). Inline
    /// `style` projects through [`crate::parse_style_intent`] → temporary
    /// [`crate::css_map::LayoutStyle`]（**禁止**在本路径新增第二套声明解析；
    /// 非产品 SoT）。Product geometry after paint: iced probe →
    /// [`LayoutBoxStore`]; pre-paint fallback: [`crate::measure_layout`].
    pub fn resolve_now(&mut self) {
        self.layout.clear();
        let w = self.logical_width.max(1.0);
        let default_row_h = 40.0;

        // Ensure html/body cover the viewport first.
        self.layout.insert(
            self.html_root.0,
            LayoutBox {
                handle: self.html_root,
                x: 0.0,
                y: 0.0,
                width: w,
                height: self.logical_height.max(1.0),
            },
        );
        self.layout.insert(
            self.mount_root.0,
            LayoutBox {
                handle: self.mount_root,
                x: 0.0,
                y: 0.0,
                width: w,
                height: self.logical_height.max(1.0),
            },
        );

        let mut stack_y = 0.0f32;
        let top_level = self.children_of(self.mount_root);
        for child in top_level {
            stack_y = self.layout_subtree(child, 0.0, stack_y, w, default_row_h);
        }
    }

    fn layout_subtree(
        &mut self,
        handle: NodeHandle,
        parent_x: f32,
        y: f32,
        avail_w: f32,
        default_row_h: f32,
    ) -> f32 {
        let is_element = matches!(
            self.nodes.get(&handle.0).map(|n| &n.data),
            Some(NodeData::Element { .. })
        );
        if !is_element {
            return y;
        }
        let intent = crate::style::parse_style_intent(self, handle);
        if intent.hidden {
            return y;
        }
        let mut box_ = LayoutBox {
            handle,
            x: parent_x,
            y: y + intent.margin_top,
            width: avail_w,
            height: default_row_h,
        };
        crate::style::apply_style_to_box(&mut box_, &intent, avail_w);
        if intent.height.is_none() {
            if let Some(text) = self.text_content(handle) {
                if !text.trim().is_empty() {
                    box_.height = box_
                        .height
                        .max(intent.font_size + intent.padding * 2.0 + 8.0);
                }
            }
        }

        let children = self.children_of(handle);
        let element_children: Vec<_> = children
            .iter()
            .copied()
            .filter(|c| self.element_tag(*c).is_some())
            .collect();

        if element_children.is_empty() {
            self.layout.insert(handle.0, box_);
            return box_.y + box_.height;
        }

        let inner_x = box_.x + intent.padding;
        let inner_w = (box_.width - intent.padding * 2.0).max(0.0);
        let inner_y = box_.y + intent.padding;

        let content_bottom = if intent.row {
            self.layout_row_children(
                &element_children,
                inner_x,
                inner_y,
                inner_w,
                intent.gap,
                default_row_h,
            )
        } else {
            let mut cursor_y = inner_y;
            for child in element_children {
                cursor_y = self.layout_subtree(child, inner_x, cursor_y, inner_w, default_row_h);
            }
            cursor_y
        } + intent.padding;

        if intent.height.is_none() {
            box_.height = (content_bottom - box_.y).max(box_.height);
            if let Some(mh) = intent.min_height {
                box_.height = box_.height.max(mh);
            }
        }
        self.layout.insert(handle.0, box_);
        box_.y + box_.height
    }

    fn layout_row_children(
        &mut self,
        children: &[NodeHandle],
        origin_x: f32,
        origin_y: f32,
        avail_w: f32,
        gap: f32,
        default_row_h: f32,
    ) -> f32 {
        if children.is_empty() {
            return origin_y;
        }
        // Match measure / iced-view: only visible children consume gap and flex.
        let visible: Vec<(NodeHandle, crate::style::StyleIntent)> = children
            .iter()
            .copied()
            .map(|c| (c, crate::style::parse_style_intent(self, c)))
            .filter(|(_, intent)| !intent.hidden)
            .collect();
        if visible.is_empty() {
            return origin_y;
        }
        let gap_total = gap * (visible.len().saturating_sub(1) as f32);
        let fixed: f32 = visible.iter().filter_map(|(_, i)| i.width).sum();
        let flex_n = visible
            .iter()
            .filter(|(_, i)| i.width.is_none())
            .count()
            .max(1) as f32;
        let flex_each = ((avail_w - fixed - gap_total) / flex_n).max(48.0);

        let mut cx = origin_x;
        let mut max_bottom = origin_y;
        for (child, cint) in &visible {
            let child_w = cint.width.unwrap_or(flex_each).clamp(0.0, avail_w);
            let bottom = self.layout_subtree(*child, cx, origin_y, child_w, default_row_h);
            max_bottom = max_bottom.max(bottom);
            cx += child_w + gap;
        }
        max_bottom
    }

    pub fn layout_box(&self, node: NodeHandle) -> Option<LayoutBox> {
        self.layout.get(&node.0).copied()
    }

    /// Replace layout cache with measured / iced-written boxes.
    ///
    /// Prefer calling this with [`LayoutBoxStore::snapshot`] after iced draws;
    /// Style-Model `measure_layout` is the headless / pre-paint fallback.
    ///
    /// Always keeps html/body covering the viewport so hit-tests still have a
    /// root surface when the forest is sparse.
    pub fn apply_layout_boxes(&mut self, boxes: &[(NodeHandle, LayoutBox)]) {
        self.layout.clear();
        let w = self.logical_width.max(1.0);
        let h = self.logical_height.max(1.0);
        self.layout.insert(
            self.html_root.0,
            LayoutBox {
                handle: self.html_root,
                x: 0.0,
                y: 0.0,
                width: w,
                height: h,
            },
        );
        self.layout.insert(
            self.mount_root.0,
            LayoutBox {
                handle: self.mount_root,
                x: 0.0,
                y: 0.0,
                width: w,
                height: h,
            },
        );
        for &(handle, box_) in boxes {
            if handle.0 == self.html_root.0 || handle.0 == self.mount_root.0 {
                continue;
            }
            self.layout.insert(handle.0, box_);
        }
    }

    pub fn snapshot_boxes(&self) -> BoxSnapshot {
        let mut boxes: Vec<LayoutBox> = self.layout.values().copied().collect();
        boxes.sort_by_key(|b| b.handle.0);
        let mut texts = Vec::new();
        let mut tags = Vec::new();
        for (&id, node) in &self.nodes {
            match &node.data {
                NodeData::Text(t) if !t.is_empty() => texts.push((NodeHandle(id), t.clone())),
                NodeData::Element { tag, .. } => tags.push((NodeHandle(id), tag.clone())),
                _ => {}
            }
        }
        texts.sort_by_key(|(h, _)| h.0);
        tags.sort_by_key(|(h, _)| h.0);
        let mut backgrounds = Vec::new();
        for (&id, _) in &self.layout {
            let handle = NodeHandle(id);
            let intent = crate::style::parse_style_intent(self, handle);
            if let Some(bg) = intent.background {
                backgrounds.push((handle, bg));
            }
        }
        backgrounds.sort_by_key(|(h, _)| h.0);
        let gpu_slots: Vec<_> = self
            .gpu_slots
            .iter()
            .map(|(&id, s)| (NodeHandle(id), s.clone()))
            .collect();
        BoxSnapshot {
            boxes,
            texts,
            tags,
            backgrounds,
            event_targets: self.event_flags.clone(),
            gpu_slots,
        }
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeHandle> {
        let iced = shared_layout_box_store().snapshot();
        let mut best: Option<(f32, NodeHandle)> = None;
        let mut consider = |box_: &LayoutBox| {
            if x >= box_.x && y >= box_.y && x < box_.x + box_.width && y < box_.y + box_.height {
                // Prefer deepest (highest y among overlaps, then highest id).
                let score = box_.y + box_.handle.0 as f32 * 0.0001;
                if best.map(|(s, _)| score >= s).unwrap_or(true) {
                    best = Some((score, box_.handle));
                }
            }
        };
        if iced.is_empty() {
            for box_ in self.layout.values() {
                consider(box_);
            }
        } else {
            for (_, box_) in &iced {
                consider(box_);
            }
            // Keep viewport roots hittable when iced forest is sparse.
            for &root in &[self.html_root, self.mount_root] {
                if let Some(box_) = self.layout.get(&root.0) {
                    consider(box_);
                }
            }
        }
        best.map(|(_, h)| h)
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

    pub fn set_focus(&mut self, node: NodeHandle) {
        self.focused = Some(node);
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    pub fn focused(&self) -> Option<NodeHandle> {
        self.focused
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn detach(&mut self, child: NodeHandle) {
        let parent = self.nodes.get(&child.0).and_then(|n| n.parent);
        if let Some(pid) = parent {
            if let Some(Node {
                data: NodeData::Element { children, .. },
                ..
            }) = self.nodes.get_mut(&pid)
            {
                children.retain(|&c| c != child.0);
            }
        }
        if let Some(node) = self.nodes.get_mut(&child.0) {
            node.parent = None;
        }
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
        let children = self.children_of(root);
        for child in children {
            self.dispose_subtree(child);
        }
        self.detach(root);
        self.nodes.remove(&root.0);
        self.layout.remove(&root.0);
        self.gpu_slots.remove(&root.0);
        self.event_flags.retain(|(id, _)| *id != root.0);
        if self.focused == Some(root) {
            self.focused = None;
        }
    }

    fn collect_preorder(&self, id: u64, out: &mut Vec<u64>) {
        out.push(id);
        if let Some(Node {
            data: NodeData::Element { children, .. },
            ..
        }) = self.nodes.get(&id)
        {
            for &c in children {
                self.collect_preorder(c, out);
            }
        }
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
    fn query_selector_finds_body_mount_root() {
        let doc = NanaTreeDocument::new(800, 600, 1.0);
        assert_eq!(doc.query_selector("body"), Some(doc.mount_root()));
        assert_eq!(doc.query_selector("html"), Some(doc.html_root()));
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
            "menu anchors must follow iced paint, not measure/synthetic"
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
        doc.resolve_now();
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
    fn row_layout_gap_and_flex_skip_hidden_children() {
        let mut doc = NanaTreeDocument::new(400, 80, 1.0);
        let mount = doc.mount_root();
        let row = doc.create_element("div");
        doc.set_attribute(
            row,
            "style",
            "display:flex;flex-direction:row;gap:10px;width:400px;height:80px",
        );
        doc.insert(row, mount, None);
        let a = doc.create_element("div");
        doc.set_attribute(a, "style", "height:40px");
        doc.insert(a, row, None);
        let hidden = doc.create_element("div");
        doc.set_attribute(hidden, "style", "display:none;width:50px;height:40px");
        doc.insert(hidden, row, None);
        let b = doc.create_element("div");
        doc.set_attribute(b, "style", "height:40px");
        doc.insert(b, row, None);
        doc.resolve_now();

        assert!(doc.layout_box(hidden).is_none());
        let a_box = doc.layout_box(a).expect("a");
        let b_box = doc.layout_box(b).expect("b");
        // Two visible flex children: (400 - 10) / 2 = 195 each.
        assert!(
            (a_box.width - 195.0).abs() < 0.01,
            "a width {}",
            a_box.width
        );
        assert!(
            (b_box.width - 195.0).abs() < 0.01,
            "b width {}",
            b_box.width
        );
        assert!((b_box.x - (a_box.x + 195.0 + 10.0)).abs() < 0.01);
    }
}
