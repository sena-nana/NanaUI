use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::sort::CompactSort as _;
use nana_ui_core::{StoreError, ViewStateStore};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::DockItemLimits;
use crate::StableNodeId;

/// Splitter hit-target thickness. Host adapters must not invent a second divider size.
pub const DOCK_DIVIDER_HIT_SIZE: f32 = nana_ui_core::space::MD;
/// Inclusive lower clamp for a split's first-child share.
pub const MIN_SPLIT_RATIO: f32 = 0.05;
/// Inclusive upper clamp for a split's first-child share.
pub const MAX_SPLIT_RATIO: f32 = 0.95;
/// One keyboard/nudge step; matches [`MIN_SPLIT_RATIO`] so product and host adapters share a step.
pub const DOCK_SPLIT_KEYBOARD_STEP: f32 = MIN_SPLIT_RATIO;

const DOCK_WORKSPACE_LAYOUT_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockDropZone {
    Left,
    Right,
    Top,
    Bottom,
    Tab,
}

/// Recursive dock tree. `ratio` is the first child's share after clamp.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DockNode {
    Item {
        #[serde(serialize_with = "serialize_arc_str")]
        id: Arc<str>,
        #[serde(skip)]
        content: Option<StableNodeId>,
    },
    Tabs {
        #[serde(serialize_with = "serialize_arc_str_vec")]
        tabs: Vec<Arc<str>>,
        #[serde(serialize_with = "serialize_arc_str")]
        active: Arc<str>,
        #[serde(skip)]
        contents: Vec<(Arc<str>, Option<StableNodeId>)>,
    },
    Split {
        axis: DockAxis,
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
}

impl<'de> Deserialize<'de> for DockNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum DockNodeDe {
            Item {
                id: String,
            },
            Tabs {
                tabs: Vec<String>,
                active: String,
            },
            Split {
                axis: DockAxis,
                ratio: f32,
                first: Box<DockNode>,
                second: Box<DockNode>,
            },
        }
        Ok(match DockNodeDe::deserialize(deserializer)? {
            DockNodeDe::Item { id } => Self::item(id, None),
            DockNodeDe::Tabs { tabs, active } => {
                let contents = tabs.iter().map(|id| (id.clone(), None)).collect::<Vec<_>>();
                Self::tabs(tabs, active, contents)
            }
            DockNodeDe::Split {
                axis,
                ratio,
                first,
                second,
            } => Self::split(axis, ratio, *first, *second),
        })
    }
}

impl DockNode {
    pub fn item(id: impl Into<Arc<str>>, content: Option<StableNodeId>) -> Self {
        Self::Item {
            id: id.into(),
            content,
        }
    }

    pub fn tabs(
        tabs: impl IntoIterator<Item = impl Into<Arc<str>>>,
        active: impl Into<Arc<str>>,
        contents: impl IntoIterator<Item = (impl Into<Arc<str>>, Option<StableNodeId>)>,
    ) -> Self {
        let tabs = tabs.into_iter().map(Into::into).collect::<Vec<_>>();
        let active = active.into();
        Self::Tabs {
            active: effective_active(&tabs, &active),
            tabs,
            contents: contents
                .into_iter()
                .map(|(id, content)| (id.into(), content))
                .collect(),
        }
    }

    pub fn split(axis: DockAxis, ratio: f32, first: DockNode, second: DockNode) -> Self {
        Self::Split {
            axis,
            ratio: clamp_ratio(ratio),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Make `content` the content of item `id`, wherever it is; whether
    /// that changed anything.
    pub fn bind_content(&mut self, id: &str, content: StableNodeId) -> bool {
        match self {
            Self::Item {
                id: item,
                content: slot,
            } if item.as_ref() == id => slot.replace(content) != Some(content),
            Self::Item { .. } => false,
            Self::Tabs { tabs, contents, .. } => {
                if let Some((_, slot)) = contents.iter_mut().find(|(tab, _)| tab.as_ref() == id) {
                    return slot.replace(content) != Some(content);
                }
                let Some(tab) = tabs.iter().find(|tab| tab.as_ref() == id) else {
                    return false;
                };
                contents.push((Arc::clone(tab), Some(content)));
                true
            }
            Self::Split { first, second, .. } => {
                let first = first.bind_content(id, content);
                second.bind_content(id, content) || first
            }
        }
    }

    /// Unbind any panel content node that is no longer alive in the world.
    pub(crate) fn clear_missing_content(
        &mut self,
        contains: &impl Fn(StableNodeId) -> bool,
    ) -> bool {
        match self {
            Self::Item { content, .. } => {
                if content.is_some_and(|id| !contains(id)) {
                    *content = None;
                    return true;
                }
                false
            }
            Self::Tabs { contents, .. } => {
                let mut changed = false;
                for (_, slot) in contents {
                    if slot.is_some_and(|id| !contains(id)) {
                        *slot = None;
                        changed = true;
                    }
                }
                changed
            }
            Self::Split { first, second, .. } => {
                let a = first.clear_missing_content(contains);
                let b = second.clear_missing_content(contains);
                a || b
            }
        }
    }

    pub fn flatten(&self) -> Vec<Arc<str>> {
        let mut ids = Vec::new();
        collect_ids(self, &mut ids);
        ids
    }

    /// Splits on the deepest path from here to a leaf, counted without
    /// recursing; see [`crate::MAX_SPLIT_DEPTH`].
    pub fn split_depth(&self) -> usize {
        let mut deepest = 0;
        let mut open = vec![(self, 0)];
        while let Some((node, depth)) = open.pop() {
            deepest = deepest.max(depth);
            if let Self::Split { first, second, .. } = node {
                open.push((first, depth + 1));
                open.push((second, depth + 1));
            }
        }
        deepest
    }

    /// Folds each split [`crate::MAX_SPLIT_DEPTH`] deep into one tab group
    /// holding every item under it, in order: no item is lost, and no walk
    /// the framework makes goes deeper. Whether anything was folded. The
    /// tree is walked and the folded splits taken apart without recursing,
    /// so an application's tree of any depth folds on any stack.
    pub fn fold_past_split_depth(&mut self) -> bool {
        if self.split_depth() <= crate::MAX_SPLIT_DEPTH {
            return false;
        }
        let mut open = vec![(self, 0)];
        while let Some((node, depth)) = open.pop() {
            match node {
                Self::Split { .. } if depth == crate::MAX_SPLIT_DEPTH => {
                    let deep = std::mem::replace(node, Self::item("", None));
                    *node = Self::into_tabs(deep);
                }
                Self::Split { first, second, .. } => {
                    open.push((first.as_mut(), depth + 1));
                    open.push((second.as_mut(), depth + 1));
                }
                Self::Item { .. } | Self::Tabs { .. } => {}
            }
        }
        true
    }

    /// Every item of `node`, in order, as one tab group (or the one item),
    /// taking each split apart by value so none drops into another.
    fn into_tabs(node: Self) -> Self {
        let mut tabs = Vec::new();
        let mut contents = Vec::new();
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            match node {
                Self::Item { id, content } => {
                    tabs.push(Arc::clone(&id));
                    contents.push((id, content));
                }
                Self::Tabs {
                    tabs: group,
                    contents: held,
                    ..
                } => {
                    tabs.extend(group);
                    contents.extend(held);
                }
                Self::Split { first, second, .. } => {
                    pending.push(*second);
                    pending.push(*first);
                }
            }
        }
        match (tabs.len(), contents.len()) {
            (1, 1) => {
                let (id, content) = contents.pop().expect("one item");
                Self::Item { id, content }
            }
            _ => {
                let active = tabs.first().cloned().unwrap_or_else(|| Arc::from(""));
                Self::tabs(tabs, active, contents)
            }
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        match self {
            Self::Item { id: item, .. } => item.as_ref() == id,
            Self::Tabs { tabs, .. } => tabs.iter().any(|tab| tab.as_ref() == id),
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    pub fn clamp_ratios(&mut self) {
        match self {
            Self::Item { .. } => {}
            Self::Tabs { tabs, active, .. } => {
                *active = effective_active(tabs, active);
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                *ratio = clamp_ratio(*ratio);
                first.clamp_ratios();
                second.clamp_ratios();
            }
        }
    }

    /// Move `dragged_id` before or after `target_id` when they share a Tabs strip.
    pub fn reorder_tab(&mut self, dragged_id: &str, target_id: &str, before: bool) -> bool {
        reorder_dock_tab(self, dragged_id, target_id, before)
    }

    /// First-child share at `path` (`[]` is this node when it is a split).
    pub fn split_ratio_at(&self, path: &[usize]) -> Option<f32> {
        if path.is_empty() {
            return match self {
                Self::Split { ratio, .. } => Some(*ratio),
                _ => None,
            };
        }
        let Self::Split { first, second, .. } = self else {
            return None;
        };
        match path[0] {
            0 => first.split_ratio_at(&path[1..]),
            1 => second.split_ratio_at(&path[1..]),
            _ => None,
        }
    }

    /// Write a clamped first-child share at `path`. Returns whether the stored ratio changed.
    pub fn set_split_ratio_at(&mut self, path: &[usize], ratio: f32) -> bool {
        if path.is_empty() {
            let Self::Split { ratio: current, .. } = self else {
                return false;
            };
            let ratio = clamp_ratio(ratio);
            if (*current - ratio).abs() <= f32::EPSILON {
                return false;
            }
            *current = ratio;
            return true;
        }
        let Self::Split { first, second, .. } = self else {
            return false;
        };
        match path[0] {
            0 => first.set_split_ratio_at(&path[1..], ratio),
            1 => second.set_split_ratio_at(&path[1..], ratio),
            _ => false,
        }
    }

    #[cfg(test)]
    pub(super) fn split_ratio(&self) -> Option<f32> {
        self.split_ratio_at(&[])
    }
}

/// Host window identity for the primary dock surface.
pub const MAIN_SURFACE_ID: &str = "0";

pub(super) const DEFAULT_FLOATING_X: f32 = 120.0;
pub(super) const DEFAULT_FLOATING_Y: f32 = 120.0;
pub(super) const DEFAULT_FLOATING_WIDTH: f32 = 360.0;
pub(super) const DEFAULT_FLOATING_HEIGHT: f32 = 280.0;

/// One floating dock window. Hosts open a window and assemble a [`Dock`] on it.
#[derive(Debug, Clone, PartialEq)]
pub struct DockFloatingSurface {
    pub id: Arc<str>,
    pub root: DockNode,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DockFloatingSurface {
    pub fn window_key(&self) -> u64 {
        dock_surface_window_key(&self.id)
    }
}

/// Deterministic window identity for a dock surface id.
///
/// Decimal strings map to themselves so [`MAIN_SURFACE_ID`] is the primary
/// window. Other ids use a stable non-zero FNV-1a key.
pub fn dock_surface_window_key(id: &str) -> u64 {
    if let Ok(parsed) = id.parse::<u64>() {
        return parsed;
    }
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if hash == 0 { 1 } else { hash }
}

/// One surface the host should mount as an `Entity<Dock>` on that window's document.
#[derive(Debug, Clone, PartialEq)]
pub struct DockSurfaceSpec {
    pub id: Arc<str>,
    pub root: DockNode,
    pub bounds: Option<(f32, f32, f32, f32)>,
}

/// Host-applied floating window effects. Window commands stay in `nana-ui`.
#[derive(Debug, Clone, PartialEq)]
pub enum DockWorkspaceEvent {
    OpenFloating(DockFloatingSurface),
    CloseFloating(Arc<str>),
    MoveFloating {
        id: Arc<str>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    FocusFloating(Arc<str>),
}

/// Product dock authority: main tree, floating surfaces, hide set, primary id.
///
/// Split ratios are mutated with [`Self::set_split_ratio`] or live pointer
/// resize (`dock_split_ratio_from_pointer`). [`Dock`] is one in-tree surface
/// projection of this workspace; `nana_ui::dock::DockController` is a host
/// adapter, not a second live dock.
///
/// Hosts should create one `Entity<Dock>` per [`DockSurfaceSpec`] on the
/// `RuntimeDocument` that owns that window, then call [`AppContext::assemble_dock`].
#[derive(Debug, Clone, PartialEq)]
pub struct DockWorkspace {
    pub main: DockNode,
    pub floating: Vec<DockFloatingSurface>,
    /// Items omitted from [`Self::surfaces`] until [`Self::show`].
    pub hidden: Vec<Arc<str>>,
    /// Center item that cannot hide, matching Gallery `gallery.primary`.
    pub primary: Option<Arc<str>>,
    pub locked: bool,
    pub monitors: HashMap<Arc<str>, String>,
    pub item_limits: HashMap<Arc<str>, DockItemLimits>,
    next_surface: u64,
}

impl DockWorkspace {
    pub fn new(mut main: DockNode) -> Self {
        main.fold_past_split_depth();
        main.clamp_ratios();
        Self {
            main,
            floating: Vec::new(),
            hidden: Vec::new(),
            primary: None,
            locked: false,
            monitors: HashMap::new(),
            item_limits: HashMap::new(),
            next_surface: 1,
        }
    }

    pub fn primary(mut self, id: impl Into<Arc<str>>) -> Self {
        self.primary = Some(id.into());
        self
    }

    pub fn surfaces(&self) -> Vec<DockSurfaceSpec> {
        let mut surfaces = Vec::with_capacity(self.floating.len() + 1);
        surfaces.push(DockSurfaceSpec {
            id: Arc::from(MAIN_SURFACE_ID),
            root: filter_hidden(&self.main, &self.hidden).unwrap_or_else(|| self.main.clone()),
            bounds: None,
        });
        surfaces.extend(self.floating.iter().filter_map(|surface| {
            Some(DockSurfaceSpec {
                id: Arc::clone(&surface.id),
                root: filter_hidden(&surface.root, &self.hidden)?,
                bounds: Some((surface.x, surface.y, surface.width, surface.height)),
            })
        }));
        surfaces
    }

    pub fn hide(&mut self, id: impl AsRef<str>) -> bool {
        if self.locked
            || !self
                .item_limits
                .get(id.as_ref())
                .copied()
                .unwrap_or_default()
                .closeable
        {
            return false;
        }
        hide_id(
            &self.workspace_ids(),
            &mut self.hidden,
            self.primary.as_deref(),
            id.as_ref(),
        )
    }

    pub fn show(&mut self, id: impl AsRef<str>) -> bool {
        show_id(&mut self.hidden, id.as_ref())
    }

    pub fn is_visible(&self, id: &str) -> bool {
        !is_hidden(&self.hidden, id)
            && (self.main.contains(id)
                || self
                    .floating
                    .iter()
                    .any(|surface| surface.root.contains(id)))
    }

    fn workspace_ids(&self) -> Vec<Arc<str>> {
        let mut ids = self.main.flatten();
        for surface in &self.floating {
            ids.extend(surface.root.flatten());
        }
        ids
    }

    pub fn float_item(&mut self, id: impl AsRef<str>) -> Option<DockWorkspaceEvent> {
        self.float_item_at(
            id,
            DEFAULT_FLOATING_X,
            DEFAULT_FLOATING_Y,
            DEFAULT_FLOATING_WIDTH,
            DEFAULT_FLOATING_HEIGHT,
        )
    }

    pub fn float_item_at(
        &mut self,
        id: impl AsRef<str>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<DockWorkspaceEvent> {
        if self.locked
            || !self
                .item_limits
                .get(id.as_ref())
                .copied()
                .unwrap_or_default()
                .floatable
        {
            return None;
        }
        if !can_hide_id(
            &self.main.flatten(),
            &self.hidden,
            self.primary.as_deref(),
            id.as_ref(),
        ) {
            return None;
        }
        let taken = extract_item(&mut self.main, id.as_ref())?;
        self.hidden.retain(|hidden| hidden.as_ref() != id.as_ref());
        let surface = DockFloatingSurface {
            id: self.allocate_surface_id(),
            root: taken,
            x: finite(x, DEFAULT_FLOATING_X),
            y: finite(y, DEFAULT_FLOATING_Y),
            width: finite(width, DEFAULT_FLOATING_WIDTH),
            height: finite(height, DEFAULT_FLOATING_HEIGHT),
        };
        self.floating.push(surface.clone());
        Some(DockWorkspaceEvent::OpenFloating(surface))
    }

    pub fn apply(&mut self, event: DockWorkspaceEvent) {
        match event {
            DockWorkspaceEvent::OpenFloating(surface) => {
                if !self
                    .floating
                    .iter()
                    .any(|existing| existing.id == surface.id)
                {
                    self.floating.push(surface);
                }
            }
            DockWorkspaceEvent::CloseFloating(id) => {
                self.floating.retain(|surface| surface.id != id);
            }
            DockWorkspaceEvent::MoveFloating {
                id,
                x,
                y,
                width,
                height,
            } => {
                if let Some(surface) = self.floating.iter_mut().find(|surface| surface.id == id) {
                    surface.x = x;
                    surface.y = y;
                    surface.width = width;
                    surface.height = height;
                }
            }
            DockWorkspaceEvent::FocusFloating(_) => {}
        }
    }

    fn allocate_surface_id(&mut self) -> Arc<str> {
        let id = Arc::<str>::from(self.next_surface.to_string());
        self.next_surface = self.next_surface.saturating_add(1);
        id
    }

    /// Mutable tree for `surface` (`MAIN_SURFACE_ID` is `main`).
    pub fn surface_root_mut(&mut self, surface: &str) -> Option<&mut DockNode> {
        if surface == MAIN_SURFACE_ID {
            Some(&mut self.main)
        } else {
            self.floating
                .iter_mut()
                .find(|item| item.id.as_ref() == surface)
                .map(|item| &mut item.root)
        }
    }

    /// Product split-ratio mutation. Host adapters must not apply a second formula.
    pub fn set_split_ratio(&mut self, surface: &str, path: &[usize], ratio: f32) -> bool {
        if self.locked {
            return false;
        }
        self.surface_root_mut(surface)
            .is_some_and(|root| root.set_split_ratio_at(path, ratio))
    }

    /// Persist the product tree using historical `DockLayout` JSON field names.
    ///
    /// Host slot contents and [`Self::primary`] are not stored. `locked` and
    /// per-surface `monitor` are retained host metadata and round-trip unchanged.
    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Restore a product tree from historical `DockLayout` JSON.
    ///
    /// Host slot contents and [`Self::primary`] are not in the JSON. Use
    /// [`Self::restore_layout_json`] to keep the current primary.
    pub fn from_layout_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }

    /// Replace the tree from JSON while keeping [`Self::primary`].
    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        let mut restored = Self::from_layout_json(value)?;
        restored.primary = self.primary.clone();
        restored.item_limits = self.item_limits.clone();
        *self = restored;
        Ok(())
    }

    /// Write this workspace into the canonical view-state namespace.
    pub fn save_to_store(&self, store: &ViewStateStore, key: &str) -> Result<(), StoreError> {
        let json = self
            .layout_json()
            .map_err(|error| StoreError::new(error.to_string()))?;
        store.save("dock", key, 1, json)
    }

    /// Restore from the canonical view-state namespace. Returns `false` when absent.
    pub fn restore_from_store(
        &mut self,
        store: &ViewStateStore,
        key: &str,
    ) -> Result<bool, StoreError> {
        let Some(restored) =
            store.restore("dock", key, 1, |raw| Self::from_layout_json(raw).ok())?
        else {
            return Ok(false);
        };
        self.restore_layout_json(
            &restored
                .layout_json()
                .map_err(|e| StoreError::new(e.to_string()))?,
        )
        .map_err(|e| StoreError::new(e.to_string()))?;
        Ok(true)
    }
}

/// Persisted floating bounds. Same fields as historical `DockLayout` JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DockBoundsPersist {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

fn serialize_arc_str<S: Serializer>(value: &Arc<str>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value)
}

fn serialize_arc_str_vec<S: Serializer>(
    values: &[Arc<str>],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(values.iter().map(AsRef::as_ref))
}

impl Serialize for DockWorkspace {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct FloatingRef<'a> {
            surface: u64,
            root: &'a DockNode,
            bounds: DockBoundsPersist,
            monitor: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct WorkspaceRef<'a> {
            version: u8,
            main: &'a DockNode,
            floating: Vec<FloatingRef<'a>>,
            #[serde(serialize_with = "serialize_arc_str_vec")]
            hidden: &'a [Arc<str>],
            locked: bool,
        }
        let floating = self
            .floating
            .iter()
            .map(|surface| FloatingRef {
                surface: dock_surface_window_key(&surface.id),
                root: &surface.root,
                bounds: DockBoundsPersist {
                    x: surface.x,
                    y: surface.y,
                    width: surface.width,
                    height: surface.height,
                },
                monitor: self.monitors.get(&surface.id).map(String::as_str),
            })
            .collect();
        WorkspaceRef {
            version: DOCK_WORKSPACE_LAYOUT_VERSION,
            main: &self.main,
            floating,
            hidden: &self.hidden,
            locked: self.locked,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DockWorkspace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct FloatingDe {
            surface: u64,
            root: DockNode,
            bounds: DockBoundsPersist,
            #[serde(default)]
            monitor: Option<String>,
        }
        #[derive(Deserialize)]
        struct WorkspaceDe {
            #[allow(dead_code)]
            version: u8,
            main: DockNode,
            #[serde(default)]
            floating: Vec<FloatingDe>,
            #[serde(default)]
            hidden: Vec<String>,
            #[serde(default)]
            locked: bool,
        }
        let persist = WorkspaceDe::deserialize(deserializer)?;
        let mut next_surface = 1_u64;
        let monitors = persist
            .floating
            .iter()
            .filter_map(|item| {
                item.monitor
                    .clone()
                    .map(|monitor| (Arc::from(item.surface.to_string()), monitor))
            })
            .collect();
        let floating = persist
            .floating
            .into_iter()
            .map(|item| {
                next_surface = next_surface.max(item.surface.saturating_add(1));
                let mut root = item.root;
                root.fold_past_split_depth();
                root.clamp_ratios();
                DockFloatingSurface {
                    id: Arc::from(item.surface.to_string()),
                    root,
                    x: item.bounds.x,
                    y: item.bounds.y,
                    width: item.bounds.width,
                    height: item.bounds.height,
                }
            })
            .collect();
        let mut workspace = DockWorkspace::new(persist.main);
        workspace.locked = persist.locked;
        workspace.monitors = monitors;
        workspace.floating = floating;
        workspace.hidden = persist.hidden.into_iter().map(Arc::from).collect();
        workspace.next_surface = next_surface;
        Ok(workspace)
    }
}
fn collect_ids(node: &DockNode, output: &mut Vec<Arc<str>>) {
    match node {
        DockNode::Item { id, .. } => output.push(Arc::clone(id)),
        DockNode::Tabs { tabs, .. } => output.extend(tabs.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_ids(first, output);
            collect_ids(second, output);
        }
    }
}

pub(super) fn collect_contents(
    node: &DockNode,
    output: &mut Vec<(Arc<str>, Option<StableNodeId>)>,
) {
    match node {
        DockNode::Item { id, content } => output.push((Arc::clone(id), *content)),
        DockNode::Tabs { contents, .. } => output.extend(contents.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_contents(first, output);
            collect_contents(second, output);
        }
    }
}

pub(super) fn host_content_ids(node: &DockNode) -> HashSet<StableNodeId> {
    let mut ids = HashSet::new();
    let mut slots = Vec::new();
    collect_contents(node, &mut slots);
    for (_, content) in slots {
        if let Some(id) = content {
            ids.insert(id);
        }
    }
    ids
}

pub(super) fn effective_active(tabs: &[Arc<str>], active: &Arc<str>) -> Arc<str> {
    if tabs.iter().any(|tab| tab == active) {
        Arc::clone(active)
    } else {
        tabs.first().cloned().unwrap_or_else(|| Arc::clone(active))
    }
}

pub fn clamp_ratio(ratio: f32) -> f32 {
    finite(ratio, 0.5).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

/// Length remaining for both children after the divider. Never negative.
pub fn dock_split_available(extent: f32) -> f32 {
    (extent - DOCK_DIVIDER_HIT_SIZE).max(0.0)
}

/// First-child and second-child lengths for a split of `extent` at `ratio`.
pub fn dock_split_child_lengths(ratio: f32, extent: f32) -> (f32, f32) {
    let available = dock_split_available(extent);
    let first = available * clamp_ratio(ratio);
    (first, (available - first).max(0.0))
}

/// Pointer resize: initial ratio plus absolute scalar delta over available length.
///
/// `available` is the split extent minus [`DOCK_DIVIDER_HIT_SIZE`]. This is the
/// only live split-ratio formula; `nana_ui::dock::DockController` must call it.
pub fn dock_split_ratio_from_pointer(
    start_ratio: f32,
    start: f32,
    position: f32,
    available: f32,
) -> f32 {
    clamp_ratio(start_ratio + (position - start) / available.max(1.0))
}

/// Keyboard/nudge: [`DOCK_SPLIT_KEYBOARD_STEP`] per unit `steps`.
pub fn dock_nudge_split_ratio(ratio: f32, steps: f32) -> f32 {
    if !steps.is_finite() || steps == 0.0 {
        return clamp_ratio(ratio);
    }
    clamp_ratio(ratio + steps * DOCK_SPLIT_KEYBOARD_STEP)
}

pub(super) fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

pub(super) fn is_hidden(hidden: &[Arc<str>], id: &str) -> bool {
    hidden.iter().any(|item| item.as_ref() == id)
}

pub(super) fn can_hide_id(
    ids: &[Arc<str>],
    hidden: &[Arc<str>],
    primary: Option<&str>,
    id: &str,
) -> bool {
    if primary == Some(id) || is_hidden(hidden, id) || !ids.iter().any(|item| item.as_ref() == id) {
        return false;
    }
    ids.iter()
        .filter(|item| !is_hidden(hidden, item.as_ref()))
        .count()
        > 1
}

pub(super) fn hide_id(
    ids: &[Arc<str>],
    hidden: &mut Vec<Arc<str>>,
    primary: Option<&str>,
    id: &str,
) -> bool {
    if !can_hide_id(ids, hidden, primary, id) {
        return false;
    }
    hidden.push(Arc::from(id));
    true
}

pub(super) fn show_id(hidden: &mut Vec<Arc<str>>, id: &str) -> bool {
    let before = hidden.len();
    hidden.retain(|item| item.as_ref() != id);
    hidden.len() != before
}

pub(super) fn filter_hidden(node: &DockNode, hidden: &[Arc<str>]) -> Option<DockNode> {
    match node {
        DockNode::Item { id, content } => {
            (!is_hidden(hidden, id)).then(|| DockNode::item(Arc::clone(id), *content))
        }
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } => {
            let visible = tabs
                .iter()
                .filter(|id| !is_hidden(hidden, id.as_ref()))
                .cloned()
                .collect::<Vec<_>>();
            match visible.as_slice() {
                [] => None,
                [_] => {
                    let only = visible.into_iter().next().expect("one visible tab");
                    let content = contents
                        .iter()
                        .find(|(id, _)| id == &only)
                        .and_then(|(_, content)| *content);
                    Some(DockNode::item(only, content))
                }
                _ => {
                    let pairs = visible
                        .iter()
                        .map(|id| {
                            let content = contents
                                .iter()
                                .find(|(tab, _)| tab == id)
                                .and_then(|(_, content)| *content);
                            (Arc::clone(id), content)
                        })
                        .collect::<Vec<_>>();
                    Some(DockNode::tabs(visible, Arc::clone(active), pairs))
                }
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match (filter_hidden(first, hidden), filter_hidden(second, hidden)) {
            (Some(first), Some(second)) => Some(DockNode::split(*axis, *ratio, first, second)),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        },
    }
}

pub(super) fn insert_dock_item(
    root: &mut DockNode,
    target: &str,
    item: DockNode,
    zone: DockDropZone,
) -> bool {
    match zone {
        DockDropZone::Tab => insert_tab(root, target, item),
        zone => insert_split(root, target, item, zone, 0),
    }
}

fn insert_tab(root: &mut DockNode, target: &str, item: DockNode) -> bool {
    let DockNode::Item { id, content } = item.clone() else {
        return false;
    };
    match root {
        DockNode::Item {
            id: current,
            content: current_content,
        } if current.as_ref() == target => {
            let current = Arc::clone(current);
            let current_content = *current_content;
            *root = DockNode::tabs(
                [Arc::clone(&current), Arc::clone(&id)],
                Arc::clone(&id),
                [(current, current_content), (id, content)],
            );
            true
        }
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } if tabs.iter().any(|tab| tab.as_ref() == target) => {
            if !tabs.iter().any(|tab| tab == &id) {
                tabs.push(Arc::clone(&id));
                contents.push((Arc::clone(&id), content));
            }
            *active = id;
            true
        }
        DockNode::Split { first, second, .. } => {
            insert_tab(first, target, item.clone()) || insert_tab(second, target, item)
        }
        _ => false,
    }
}

pub(super) fn reorder_dock_tab(
    root: &mut DockNode,
    dragged_id: &str,
    target_id: &str,
    before: bool,
) -> bool {
    match root {
        DockNode::Tabs { tabs, contents, .. } => {
            if !reorder_sibling_ids(tabs, dragged_id, target_id, before) {
                return false;
            }
            contents.compact_sort_by_key(|(id, _)| {
                tabs.iter().position(|tab| tab == id).unwrap_or(usize::MAX)
            });
            true
        }
        DockNode::Split { first, second, .. } => {
            reorder_dock_tab(first, dragged_id, target_id, before)
                || reorder_dock_tab(second, dragged_id, target_id, before)
        }
        DockNode::Item { .. } => false,
    }
}

fn reorder_sibling_ids(
    ids: &mut Vec<Arc<str>>,
    dragged_id: &str,
    target_id: &str,
    before: bool,
) -> bool {
    if dragged_id == target_id {
        return false;
    }
    let Some(from) = ids.iter().position(|id| id.as_ref() == dragged_id) else {
        return false;
    };
    if !ids.iter().any(|id| id.as_ref() == target_id) {
        return false;
    }
    let item = ids.remove(from);
    let target = ids
        .iter()
        .position(|id| id.as_ref() == target_id)
        .expect("target remains after removing dragged sibling");
    let insert_at = if before { target } else { target + 1 };
    let changed = from != insert_at;
    ids.insert(insert_at, item);
    changed
}

/// Splits the leaf holding `target`, `depth` splits below the root, to put
/// `item` beside it. Refused at [`crate::MAX_SPLIT_DEPTH`].
fn insert_split(
    root: &mut DockNode,
    target: &str,
    item: DockNode,
    zone: DockDropZone,
    depth: usize,
) -> bool {
    if root.contains(target) && matches!(root, DockNode::Item { .. } | DockNode::Tabs { .. }) {
        if zone != DockDropZone::Tab && depth >= crate::MAX_SPLIT_DEPTH {
            return false;
        }
        let previous = root.clone();
        let (axis, first, second) = match zone {
            DockDropZone::Left => (DockAxis::Horizontal, item, previous),
            DockDropZone::Right => (DockAxis::Horizontal, previous, item),
            DockDropZone::Top => (DockAxis::Vertical, item, previous),
            DockDropZone::Bottom => (DockAxis::Vertical, previous, item),
            DockDropZone::Tab => return insert_tab(root, target, item),
        };
        *root = DockNode::split(axis, 0.5, first, second);
        return true;
    }
    match root {
        DockNode::Split { first, second, .. } => {
            insert_split(first, target, item.clone(), zone, depth + 1)
                || insert_split(second, target, item, zone, depth + 1)
        }
        _ => false,
    }
}

fn has_single_tab_group(node: &DockNode) -> bool {
    match node {
        DockNode::Item { .. } => false,
        DockNode::Tabs { tabs, .. } => tabs.len() <= 1,
        DockNode::Split { first, second, .. } => {
            has_single_tab_group(first) || has_single_tab_group(second)
        }
    }
}

pub(super) fn visible_branch<'a>(
    node: &'a DockNode,
    hidden: &[Arc<str>],
) -> Option<Cow<'a, DockNode>> {
    if hidden.is_empty() && !has_single_tab_group(node) {
        Some(Cow::Borrowed(node))
    } else {
        filter_hidden(node, hidden).map(Cow::Owned)
    }
}
pub(super) fn set_split_ratio_for_first_ids(
    node: &mut DockNode,
    first_ids: &[Arc<str>],
    ratio: f32,
) -> bool {
    match node {
        DockNode::Split {
            first,
            second,
            ratio: current,
            ..
        } => {
            if set_split_ratio_for_first_ids(first, first_ids, ratio)
                || set_split_ratio_for_first_ids(second, first_ids, ratio)
            {
                return true;
            }
            let first_flat = first.flatten();
            let owns_first = !first_ids.is_empty()
                && first_ids
                    .iter()
                    .all(|id| first_flat.iter().any(|item| item == id))
                && first_ids.iter().all(|id| !second.contains(id.as_ref()));
            if !owns_first {
                return false;
            }
            let next = clamp_ratio(ratio);
            if (*current - next).abs() <= f32::EPSILON {
                return false;
            }
            *current = next;
            true
        }
        _ => false,
    }
}

pub(super) fn extract_item(root: &mut DockNode, id: &str) -> Option<DockNode> {
    let (remaining, taken) = take_item_node(root.clone(), id);
    let (Some(remaining), Some(taken)) = (remaining, taken) else {
        return None;
    };
    *root = remaining;
    Some(taken)
}

fn take_item_node(node: DockNode, id: &str) -> (Option<DockNode>, Option<DockNode>) {
    match node {
        DockNode::Item { id: item, content } => {
            if item.as_ref() == id {
                (None, Some(DockNode::Item { id: item, content }))
            } else {
                (Some(DockNode::Item { id: item, content }), None)
            }
        }
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } => {
            if !tabs.iter().any(|tab| tab.as_ref() == id) {
                return (
                    Some(DockNode::Tabs {
                        tabs,
                        active,
                        contents,
                    }),
                    None,
                );
            }
            let taken_content = contents
                .iter()
                .find(|(tab, _)| tab.as_ref() == id)
                .and_then(|(_, content)| *content);
            let taken = DockNode::item(Arc::<str>::from(id), taken_content);
            let tabs = tabs
                .into_iter()
                .filter(|tab| tab.as_ref() != id)
                .collect::<Vec<_>>();
            let contents = contents
                .into_iter()
                .filter(|(tab, _)| tab.as_ref() != id)
                .collect::<Vec<_>>();
            let remaining = match tabs.as_slice() {
                [] => None,
                [_] => {
                    let only = tabs.into_iter().next().expect("one remaining tab");
                    let content = contents
                        .into_iter()
                        .find(|(tab, _)| tab == &only)
                        .and_then(|(_, content)| content);
                    Some(DockNode::item(only, content))
                }
                _ => Some(DockNode::Tabs {
                    active: effective_active(&tabs, &active),
                    tabs,
                    contents,
                }),
            };
            (remaining, Some(taken))
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match take_item_node(*first, id) {
            (remaining_first, Some(taken)) => {
                let remaining = match remaining_first {
                    Some(first) => Some(DockNode::split(axis, ratio, first, *second)),
                    None => Some(*second),
                };
                (remaining, Some(taken))
            }
            (Some(first), None) => match take_item_node(*second, id) {
                (remaining_second, Some(taken)) => {
                    let remaining = match remaining_second {
                        Some(second) => Some(DockNode::split(axis, ratio, first, second)),
                        None => Some(first),
                    };
                    (remaining, Some(taken))
                }
                (Some(second), None) => (Some(DockNode::split(axis, ratio, first, second)), None),
                (None, None) => (Some(first), None),
            },
            (None, None) => take_item_node(*second, id),
        },
    }
}
pub(super) fn activate_runtime_dock_tab(node: &mut DockNode, id: &str) -> bool {
    match node {
        DockNode::Item { id: item, .. } => item.as_ref() == id,
        DockNode::Tabs { tabs, active, .. } => {
            if tabs.iter().any(|tab| tab.as_ref() == id) {
                *active = Arc::from(id);
                true
            } else {
                false
            }
        }
        DockNode::Split { first, second, .. } => {
            activate_runtime_dock_tab(first, id) || activate_runtime_dock_tab(second, id)
        }
    }
}
