//! Flattened disclosure tree. Application owns node identities and expansion.
//!
//! With [`TreeView::multiple`] the tree keeps a selection set with an active
//! row (#342): a press selects one row, Ctrl (Cmd) toggles a row in or out,
//! Shift selects the range from the anchor, Shift with the arrows extends it;
//! every change emits [`TreeSelectionChanged`]. [`TreeView::columns`] puts on
//! / off icons at the end of each row (visible, locked): a press flips one
//! without touching the selection, and dragging across rows sets the same
//! value on each ([`TreeToggleChanged`]). [`TreeView::reorderable`] lets rows
//! be dragged onto another row — before it, into it or after it, with a
//! preview — and reports a [`TreeDropRequested`] for the application to
//! accept or not. [`TreeView::renamable`] opens an in-place text input on F2
//! or a double click: Enter or leaving commits ([`TreeRenameRequested`]),
//! Escape cancels.
//!
//! For assistive technology the tree is a `Tree` named by
//! [`TreeView::label`], and each visible row is a virtual `TreeItem` under it
//! (#349, [`crate::virtual_tree_item_id`]) with its level, expansion and
//! selection. Click, Focus, Expand and Collapse on a row do what a click, the
//! arrow keys and the disclosure do. Each of a row's toggles is a virtual
//! `Checkbox` under the row (#353, [`crate::virtual_tree_toggle_id`]) named
//! by its column; Click on it does what a press on the toggle does.

use std::sync::Arc;

use nana_ui_core::{
    ControlSize, Icon, LengthSpec, PositionSpec, TreeNavigation, TreeNode, TreeViewEvent,
    tree_navigation_event,
};

use crate::view_components::{TextChanged, TextInput, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentTextRegion, ComponentView,
    DocumentId, Entity, FocusWithinChanged, FrameworkError, InteractionState, LayoutBox,
    MutationQueue, NodeKind, NodePointer, NodePointerHooks, NodeStyle, StableNodeId,
    StandardVisual, UiWorld,
};

/// A press that moves less than this is a click, px.
const DRAG_SLOP: f32 = nana_ui_core::space::XS;

/// Placement resolved for a tree drop target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeDropPosition {
    Before,
    Inside,
    After,
}

/// Framework-owned tree drop intent. Applications keep node semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDropIntent {
    pub target: Arc<str>,
    pub position: TreeDropPosition,
}

/// A column of on / off icons at the end of every row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeToggleColumn {
    pub icon: Icon,
    /// What the column means, for assistive technology: `Visible`.
    pub label: Arc<str>,
}

impl TreeToggleColumn {
    pub fn new(icon: Icon, label: impl Into<Arc<str>>) -> Self {
        Self {
            icon,
            label: label.into(),
        }
    }
}

/// The selection changed: every selected row in tree order, and the active
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSelectionChanged {
    pub selected: Vec<Arc<str>>,
    pub active: Option<Arc<str>>,
}

/// A row's toggle in `column` was set to `on`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeToggleChanged {
    pub id: Arc<str>,
    pub column: usize,
    pub on: bool,
}

/// `rows` were dropped at `intent`. The tree does not move them: the
/// application changes its nodes if it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDropRequested {
    pub rows: Vec<Arc<str>>,
    pub intent: TreeDropIntent,
}

/// `id` was renamed to `text`. The application renames the node if it
/// accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRenameRequested {
    pub id: Arc<str>,
    pub text: String,
}

/// A press held on the tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TreeGesture {
    /// On a row: a click, or the start of a drag.
    Press {
        row: Arc<str>,
        origin: (f32, f32),
        /// The row was already selected: a click without a drag selects it
        /// alone on release, a drag takes the whole selection.
        select_on_release: bool,
        dragging: bool,
    },
    /// Across a toggle column, setting each row to `on`.
    Swipe {
        column: usize,
        on: bool,
        touched: Vec<Arc<str>>,
    },
}

/// Visible-row tree. Hidden collapsed children are not projected.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeView {
    pub nodes: Vec<TreeNode<Arc<str>>>,
    pub size: ControlSize,
    pub style: NodeStyle,
    /// Ctrl and Shift build a selection of several rows.
    pub multiple: bool,
    /// On / off icons at the end of each row; a node's states are its
    /// [`TreeNode::toggles`].
    pub columns: Vec<TreeToggleColumn>,
    /// Rows drag onto other rows.
    pub reorderable: bool,
    /// F2 or a double click renames a row in place.
    pub renamable: bool,
    /// The tree's name for assistive technology: `Layers`.
    pub label: Option<Arc<str>>,
    pub(crate) active: Option<Arc<str>>,
    pub(crate) anchor: Option<Arc<str>>,
    pub(crate) gesture: Option<TreeGesture>,
    pub(crate) drop: Option<TreeDropIntent>,
    pub(crate) renaming: Option<Arc<str>>,
    /// What the rename input holds, for a commit by leaving.
    pub(crate) rename_text: String,
    pub(crate) rename_input: Option<StableNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRowData {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub icon: Option<Icon>,
    pub depth: u8,
    pub branch: bool,
    pub expanded: bool,
    pub selected: bool,
    pub disabled: bool,
    pub toggles: Vec<bool>,
    /// Columns this row has no toggle in; see [`TreeNode::hidden_toggles`].
    pub hidden_toggles: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRowGeometry {
    pub id: Arc<str>,
    pub bounds: LayoutBox,
    pub disclosure: Option<LayoutBox>,
    pub icon: Option<(Icon, LayoutBox, [f32; 4])>,
    pub label: ComponentTextRegion,
    pub selected: bool,
    pub disabled: bool,
    pub expanded: bool,
    pub background: Option<[f32; 4]>,
    /// The toggle icons at the end of the row, first column first.
    pub toggles: Vec<(Icon, LayoutBox, [f32; 4])>,
}

/// Where a drop would land: a line between rows, or a row outlined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeDropMark {
    pub bounds: LayoutBox,
    pub inside: bool,
    pub color: [f32; 4],
}

impl TreeView {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(nodes: impl IntoIterator<Item = TreeNode<Arc<str>>>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            size: ControlSize::Small,
            style: NodeStyle::default(),
            multiple: false,
            columns: Vec::new(),
            reorderable: false,
            renamable: false,
            label: None,
            active: None,
            anchor: None,
            gesture: None,
            drop: None,
            renaming: None,
            rename_text: String::new(),
            rename_input: None,
        }
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    pub fn columns(mut self, columns: impl IntoIterator<Item = TreeToggleColumn>) -> Self {
        self.columns = columns.into_iter().collect();
        self
    }

    pub fn reorderable(mut self, reorderable: bool) -> Self {
        self.reorderable = reorderable;
        self
    }

    pub fn renamable(mut self, renamable: bool) -> Self {
        self.renamable = renamable;
        self
    }

    /// Name the tree for assistive technology. Rows are named by their
    /// labels.
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn visible_rows(&self) -> Vec<TreeRowData> {
        let mut rows = Vec::new();
        collect_rows(&self.nodes, 0, &mut rows);
        rows
    }

    /// The first selected row in tree order.
    pub fn selected_id(&self) -> Option<&Arc<str>> {
        find_selected(&self.nodes)
    }

    /// Every selected row, in tree order.
    pub fn selected_ids(&self) -> Vec<Arc<str>> {
        let mut out = Vec::new();
        collect_selected(&self.nodes, &mut out);
        out
    }

    /// The row the selection is about: the last one clicked, if it is
    /// still selected, else the first selected.
    pub fn active_id(&self) -> Option<&Arc<str>> {
        self.active
            .as_ref()
            .filter(|id| is_selected(&self.nodes, id))
            .or_else(|| self.selected_id())
    }

    /// The row being renamed.
    pub fn renaming(&self) -> Option<&Arc<str>> {
        self.renaming.as_ref()
    }

    pub fn apply_event(&mut self, event: TreeViewEvent<Arc<str>>) -> bool {
        match event {
            TreeViewEvent::Toggle(id) => toggle_node(&mut self.nodes, &id),
            TreeViewEvent::Select(id) => {
                self.active = Some(Arc::clone(&id));
                self.anchor = Some(Arc::clone(&id));
                select_node(&mut self.nodes, &id)
            }
        }
    }

    pub fn navigate(&mut self, navigation: TreeNavigation) -> Option<TreeViewEvent<Arc<str>>> {
        let from = self.active_id().cloned();
        let event = tree_navigation_event(&self.nodes, from.as_ref(), navigation)?;
        self.apply_event(event.clone());
        Some(event)
    }

    /// Select only `id`; it becomes active and the range anchor.
    pub fn select_only(&mut self, id: &Arc<str>) -> bool {
        self.apply_event(TreeViewEvent::Select(Arc::clone(id)))
    }

    /// Add `id` to the selection or take it out (Ctrl / Cmd); it becomes
    /// active and the anchor.
    pub fn toggle_in_selection(&mut self, id: &Arc<str>) -> bool {
        let on = !is_selected(&self.nodes, id);
        self.active = Some(Arc::clone(id));
        self.anchor = Some(Arc::clone(id));
        set_selected(&mut self.nodes, id, on)
    }

    /// Select the visible rows from the anchor to `id` (Shift); `id` becomes
    /// active, the anchor stays.
    pub fn select_range_to(&mut self, id: &Arc<str>) -> bool {
        let rows = self.visible_rows();
        let to = rows.iter().position(|row| &row.id == id);
        let from = self
            .anchor
            .as_ref()
            .and_then(|anchor| rows.iter().position(|row| &row.id == anchor))
            .or(to);
        let (Some(from), Some(to)) = (from, to) else {
            return false;
        };
        let (low, high) = (from.min(to), from.max(to));
        let range = rows[low..=high]
            .iter()
            .map(|row| Arc::clone(&row.id))
            .collect::<Vec<_>>();
        if self.anchor.is_none() {
            self.anchor = Some(Arc::clone(id));
        }
        self.active = Some(Arc::clone(id));
        set_selection(&mut self.nodes, &range)
    }

    /// Move the active row by `navigation` and select the range from the
    /// anchor to it (Shift with the arrows).
    pub fn extend(&mut self, navigation: TreeNavigation) -> bool {
        let from = self.active_id().cloned();
        let Some(TreeViewEvent::Select(to)) =
            tree_navigation_event(&self.nodes, from.as_ref(), navigation)
        else {
            return false;
        };
        if self.anchor.is_none() {
            self.anchor = from;
        }
        self.select_range_to(&to)
    }

    fn selection_event(&self) -> TreeSelectionChanged {
        TreeSelectionChanged {
            selected: self.selected_ids(),
            active: self.active_id().cloned(),
        }
    }

    fn intrinsic_height(&self, metrics: nana_ui_core::ThemeMetrics) -> f32 {
        let count = self.visible_rows().len().max(1) as f32;
        count * self.size.height_in(metrics) + (count - 1.0) * metrics.tree.row_gap
    }
}

impl ComponentView for TreeView {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activate_at: Some(crate::AppContext::activate_tree_at),
        assembler: Some(AppContext::assemble_tree_rename),
        hooks: Some(|hooks| hooks.tree = Some(&TREE_HOOKS)),
        pointer: Some(&TREE_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // a gesture, a rename and the active row are the tree's own
        next.active = self.active.take();
        next.anchor = self.anchor.take();
        next.gesture = self.gesture.take();
        next.drop = self.drop.take();
        next.renaming = self.renaming.take();
        next.rename_text = std::mem::take(&mut self.rename_text);
        next.rename_input = self.rename_input.take();
        *self = next;
    }

    fn keep_framework_state(&mut self, previous: &Self) {
        if self.rename_input.is_none() {
            self.rename_input = previous.rename_input;
        }
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tree".into() }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let rows = self.visible_rows();
        let visual = StandardVisual::TreeView {
            rows: rows.into(),
            size: self.size,
            columns: self.columns.iter().map(|column| column.icon).collect(),
            column_labels: self
                .columns
                .iter()
                .map(|column| Arc::clone(&column.label))
                .collect(),
            drop: self.drop.clone(),
            active: self.active_id().cloned(),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::Fill);
        }
        // the rename input is placed over its row
        if layout.position == PositionSpec::Static {
            layout.position = PositionSpec::Relative;
        }
        // Boxes are border-box, so declared padding has to be added on top of
        // the row stack; otherwise it eats the last row. A caller that sets its
        // own height (a tree inside a fixed viewport) keeps it.
        let padding = layout.resolved_padding();
        let content = self.intrinsic_height(world.theme_metrics()) + padding.top + padding.bottom;
        if layout.height.is_none() {
            layout.height = Some(LengthSpec::Px(content));
        }
        if layout.min_height.is_none() {
            layout.min_height = Some(LengthSpec::Px(content));
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Tree,
                label: self.label.clone(),
                disabled: false,
                ..AccessibilityState::default()
            },
        );
    }
}

fn collect_rows(nodes: &[TreeNode<Arc<str>>], depth: u8, rows: &mut Vec<TreeRowData>) {
    for node in nodes {
        rows.push(TreeRowData {
            id: Arc::clone(&node.id),
            label: Arc::from(node.label.as_str()),
            icon: node.icon,
            depth,
            branch: node.branch,
            expanded: node.expanded,
            selected: node.selected,
            disabled: node.disabled,
            toggles: node.toggles.clone(),
            hidden_toggles: node.hidden_toggles,
        });
        if node.branch && node.expanded {
            collect_rows(&node.children, depth.saturating_add(1), rows);
        }
    }
}

fn find_selected(nodes: &[TreeNode<Arc<str>>]) -> Option<&Arc<str>> {
    for node in nodes {
        if node.selected {
            return Some(&node.id);
        }
        if let Some(id) = find_selected(&node.children) {
            return Some(id);
        }
    }
    None
}

fn collect_selected(nodes: &[TreeNode<Arc<str>>], out: &mut Vec<Arc<str>>) {
    for node in nodes {
        if node.selected {
            out.push(Arc::clone(&node.id));
        }
        collect_selected(&node.children, out);
    }
}

fn find_node<'a>(nodes: &'a [TreeNode<Arc<str>>], id: &Arc<str>) -> Option<&'a TreeNode<Arc<str>>> {
    for node in nodes {
        if &node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node(&node.children, id) {
            return Some(found);
        }
    }
    None
}

fn find_node_mut<'a>(
    nodes: &'a mut [TreeNode<Arc<str>>],
    id: &Arc<str>,
) -> Option<&'a mut TreeNode<Arc<str>>> {
    for node in nodes {
        if &node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

fn is_selected(nodes: &[TreeNode<Arc<str>>], id: &Arc<str>) -> bool {
    find_node(nodes, id).is_some_and(|node| node.selected)
}

/// `inner` is `outer` or one of its descendants.
fn is_within(nodes: &[TreeNode<Arc<str>>], outer: &Arc<str>, inner: &Arc<str>) -> bool {
    find_node(nodes, outer)
        .is_some_and(|node| &node.id == inner || find_node(&node.children, inner).is_some())
}

fn set_selected(nodes: &mut [TreeNode<Arc<str>>], id: &Arc<str>, on: bool) -> bool {
    match find_node_mut(nodes, id) {
        Some(node) if !node.disabled && node.selected != on => {
            node.selected = on;
            true
        }
        _ => false,
    }
}

fn set_selection(nodes: &mut [TreeNode<Arc<str>>], ids: &[Arc<str>]) -> bool {
    let mut changed = false;
    for node in nodes {
        let selected = !node.disabled && ids.contains(&node.id);
        if node.selected != selected {
            node.selected = selected;
            changed = true;
        }
        changed |= set_selection(&mut node.children, ids);
    }
    changed
}

fn toggle_node(nodes: &mut [TreeNode<Arc<str>>], id: &Arc<str>) -> bool {
    for node in nodes {
        if &node.id == id {
            if !node.branch {
                return false;
            }
            node.expanded = !node.expanded;
            return true;
        }
        if toggle_node(&mut node.children, id) {
            return true;
        }
    }
    false
}

fn select_node(nodes: &mut [TreeNode<Arc<str>>], id: &Arc<str>) -> bool {
    let mut changed = false;
    for node in nodes {
        let selected = &node.id == id && !node.disabled;
        if node.selected != selected {
            node.selected = selected;
            changed = true;
        }
        changed |= select_node(&mut node.children, id);
    }
    changed
}

pub(crate) fn tree_view_geometry(
    bounds: LayoutBox,
    rows: &[TreeRowData],
    size: ControlSize,
    columns: &[Icon],
    drop: Option<&TreeDropIntent>,
    palette: &nana_ui_core::SemanticPalette,
    metrics: nana_ui_core::ThemeMetrics,
) -> crate::ComponentGeometry {
    let row_height = size.height_in(metrics);
    let tree = metrics.tree;
    // the toggle columns take the row's end, one icon and one inset each
    let toggles_width = columns.len() as f32 * (tree.icon + tree.inset);
    let geometry: Vec<TreeRowGeometry> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let y = bounds.y + index as f32 * (row_height + tree.row_gap);
            let row_bounds = LayoutBox {
                x: bounds.x,
                y,
                width: bounds.width,
                height: row_height,
            };
            let indent = f32::from(row.depth) * tree.indent;
            let mut cursor = bounds.x + indent + tree.inset;
            let disclosure = row.branch.then(|| {
                let box_ = LayoutBox {
                    x: cursor,
                    y: y + (row_height - tree.disclosure) / 2.0,
                    width: tree.disclosure,
                    height: tree.disclosure,
                };
                cursor += tree.disclosure;
                box_
            });
            let icon = row.icon.map(|icon| {
                let box_ = LayoutBox {
                    x: cursor,
                    y: y + (row_height - tree.icon) / 2.0,
                    width: tree.icon,
                    height: tree.icon,
                };
                cursor += tree.icon + tree.inset;
                (
                    icon,
                    box_,
                    if row.disabled {
                        palette.faint.as_rgba_array()
                    } else {
                        palette.muted.as_rgba_array()
                    },
                )
            });
            let toggles_x = bounds.x + bounds.width - toggles_width;
            let toggles = columns
                .iter()
                .enumerate()
                .map(|(column, icon)| {
                    let on = row.toggles.get(column).copied().unwrap_or(false);
                    // A hidden cell keeps its column index with an empty box:
                    // nothing is drawn or hit there.
                    let side = if nana_ui_core::tree::toggle_hidden(row.hidden_toggles, column) {
                        0.0
                    } else {
                        tree.icon
                    };
                    (
                        *icon,
                        LayoutBox {
                            x: toggles_x + column as f32 * (tree.icon + tree.inset),
                            y: y + (row_height - tree.icon) / 2.0,
                            width: side,
                            height: side,
                        },
                        if on {
                            palette.muted.as_rgba_array()
                        } else {
                            palette.faint.as_rgba_array()
                        },
                    )
                })
                .collect();
            TreeRowGeometry {
                id: Arc::clone(&row.id),
                bounds: row_bounds,
                disclosure,
                icon,
                label: ComponentTextRegion {
                    bounds: LayoutBox {
                        x: cursor,
                        y,
                        width: (bounds.x + bounds.width - toggles_width - cursor - tree.inset)
                            .max(0.0),
                        height: row_height,
                    },
                    content: Arc::clone(&row.label).into(),
                    color: Some(if row.disabled {
                        palette.faint.as_rgba_array()
                    } else {
                        palette.text.as_rgba_array()
                    }),
                    font_size: size.text_size(),
                    font_weight: row.selected.then_some(600),
                },
                selected: row.selected,
                disabled: row.disabled,
                expanded: row.expanded,
                background: row.selected.then_some(palette.selected.as_rgba_array()),
                toggles,
            }
        })
        .collect();
    let drop = drop.and_then(|intent| {
        let row = geometry.iter().find(|row| row.id == intent.target)?;
        let line = nana_ui_core::HAIRLINE * 2.0;
        let bounds = match intent.position {
            TreeDropPosition::Before => LayoutBox {
                y: row.bounds.y - (tree.row_gap + line) / 2.0,
                height: line,
                ..row.bounds
            },
            TreeDropPosition::After => LayoutBox {
                y: row.bounds.y + row.bounds.height + (tree.row_gap - line) / 2.0,
                height: line,
                ..row.bounds
            },
            TreeDropPosition::Inside => row.bounds,
        };
        Some(TreeDropMark {
            bounds,
            inside: intent.position == TreeDropPosition::Inside,
            color: palette.accent.as_rgba_array(),
        })
    });
    crate::ComponentGeometry::TreeView {
        rows: geometry,
        drop,
    }
}

pub(crate) fn tree_row_at(rows: &[TreeRowGeometry], x: f32, y: f32) -> Option<usize> {
    rows.iter().position(|row| row.bounds.contains(x, y))
}

pub(crate) fn tree_disclosure_at(rows: &[TreeRowGeometry], x: f32, y: f32) -> Option<usize> {
    rows.iter().position(|row| {
        row.disclosure
            .is_some_and(|disclosure| disclosure.contains(x, y))
    })
}

/// The toggle cell at `(x, y)`: the row's full height around each icon.
fn tree_toggle_at(rows: &[TreeRowGeometry], x: f32, y: f32) -> Option<(usize, usize)> {
    let row = tree_row_at(rows, x, y)?;
    let cells = &rows[row].toggles;
    let column = cells.iter().position(|(_, icon, _)| {
        let reach = (rows[row].bounds.height - icon.height).max(0.0) / 2.0;
        icon.width > 0.0 && x >= icon.x - reach && x < icon.x + icon.width + reach
    })?;
    Some((row, column))
}

/// The router reaches this picker's keyboard only once one exists.
pub(crate) static TREE_HOOKS: crate::framework::NavigateHooks<crate::TreeNavigation> =
    crate::framework::NavigateHooks {
        navigate: crate::AppContext::navigate_focused_tree,
    };

const TREE_POINTER: NodePointerHooks<TreeView> = NodePointerHooks {
    moved: tree_moved,
    left: |cx, tree| {
        // a cancelled press: nothing lands
        cx.update_component(tree, |tree, _| {
            tree.gesture = None;
            tree.drop = None;
        })
    },
    pressed: tree_pressed,
    released: tree_released,
    wheel: |_, _, _| Ok(false),
};

/// The tree's rows as laid out, and the pointer in layout px.
fn tree_rows(
    cx: &AppContext,
    tree: Entity<TreeView>,
    pointer: &NodePointer,
) -> Option<(Vec<TreeRowGeometry>, (f32, f32))> {
    let crate::ComponentGeometry::TreeView { rows, .. } =
        cx.world().component_geometry(tree.stable_id())?
    else {
        return None;
    };
    let origin = cx.world().component_layout_box(tree.stable_id())?;
    Some((rows, (pointer.x + origin.x, pointer.y + origin.y)))
}

fn tree_pressed(
    cx: &mut AppContext,
    tree: Entity<TreeView>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    let Some((rows, (x, y))) = tree_rows(cx, tree, &pointer) else {
        return Ok(false);
    };
    if let Some(index) = tree_disclosure_at(&rows, x, y) {
        let id = Arc::clone(&rows[index].id);
        cx.update_component(tree, |tree, view| {
            apply_tree_event(tree, view, TreeViewEvent::Toggle(id))
        })?;
        return Ok(true);
    }
    if let Some((index, column)) = tree_toggle_at(&rows, x, y) {
        let id = Arc::clone(&rows[index].id);
        cx.update_component(tree, |tree, view| {
            let on = flip_toggle(tree, view, &id, column);
            tree.gesture = Some(TreeGesture::Swipe {
                column,
                on,
                touched: vec![id],
            });
        })?;
        return Ok(true);
    }
    let Some(index) = tree_row_at(&rows, x, y) else {
        return Ok(false);
    };
    if rows[index].disabled {
        return Ok(true);
    }
    let id = Arc::clone(&rows[index].id);
    let rename = pointer.click_count >= 2;
    let modifiers = pointer.modifiers;
    let primary = modifiers.control || modifiers.meta;
    cx.update_component(tree, |tree, view| {
        let before = tree.selected_ids();
        let already = is_selected(&tree.nodes, &id);
        let mut select_on_release = false;
        if tree.multiple && primary {
            tree.toggle_in_selection(&id);
        } else if tree.multiple && modifiers.shift {
            tree.select_range_to(&id);
        } else if already && tree.reorderable && before.len() > 1 {
            // a drag takes the whole selection; a click selects this alone
            select_on_release = true;
            tree.active = Some(Arc::clone(&id));
        } else if tree.select_only(&id) {
            view.emit(TreeViewEvent::Select(Arc::clone(&id)));
        } else {
            tree.active = Some(Arc::clone(&id));
        }
        if tree.selected_ids() != before {
            view.emit(tree.selection_event());
        }
        tree.gesture = Some(TreeGesture::Press {
            row: Arc::clone(&id),
            origin: (x, y),
            select_on_release,
            dragging: false,
        });
    })?;
    if rename && cx.read(tree, |tree| tree.renamable)? {
        cx.update_component(tree, |tree, _| tree.gesture = None)?;
        cx.begin_tree_rename(tree, &id)?;
    }
    Ok(true)
}

fn tree_moved(
    cx: &mut AppContext,
    tree: Entity<TreeView>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    let Some((rows, (x, y))) = tree_rows(cx, tree, &pointer) else {
        return Ok(true);
    };
    let over = tree_row_at(&rows, x, y);
    cx.update_component(tree, |tree, view| match tree.gesture.clone() {
        Some(TreeGesture::Swipe {
            column,
            on,
            mut touched,
        }) => {
            if let Some(index) = over
                && !touched.contains(&rows[index].id)
            {
                let id = Arc::clone(&rows[index].id);
                if let Some(event) = set_toggle(tree, &id, column, on) {
                    view.emit(event);
                }
                touched.push(id);
                tree.gesture = Some(TreeGesture::Swipe {
                    column,
                    on,
                    touched,
                });
            }
        }
        Some(TreeGesture::Press {
            row,
            origin,
            select_on_release,
            dragging,
        }) => {
            if !tree.reorderable {
                return;
            }
            let moved = (x - origin.0).abs().max((y - origin.1).abs()) >= DRAG_SLOP;
            if !dragging && !moved {
                return;
            }
            tree.gesture = Some(TreeGesture::Press {
                row: Arc::clone(&row),
                origin,
                select_on_release,
                dragging: true,
            });
            let dragged = dragged_rows(tree, &row);
            tree.drop = over.and_then(|index| {
                let target = &rows[index];
                if target.disabled
                    || dragged
                        .iter()
                        .any(|dragged| is_within(&tree.nodes, dragged, &target.id))
                {
                    return None;
                }
                let fraction = (y - target.bounds.y) / target.bounds.height.max(1.0);
                let position = if fraction < 0.25 {
                    TreeDropPosition::Before
                } else if fraction > 0.75 {
                    TreeDropPosition::After
                } else {
                    TreeDropPosition::Inside
                };
                Some(TreeDropIntent {
                    target: Arc::clone(&target.id),
                    position,
                })
            });
        }
        None => {}
    })?;
    Ok(true)
}

fn tree_released(
    cx: &mut AppContext,
    tree: Entity<TreeView>,
    _: NodePointer,
) -> Result<bool, FrameworkError> {
    cx.update_component(tree, |tree, view| {
        let gesture = tree.gesture.take();
        let drop = tree.drop.take();
        let Some(TreeGesture::Press {
            row,
            select_on_release,
            dragging,
            ..
        }) = gesture
        else {
            return;
        };
        if dragging {
            if let Some(intent) = drop {
                view.emit(TreeDropRequested {
                    rows: dragged_rows(tree, &row),
                    intent,
                });
            }
        } else if select_on_release {
            select_alone(tree, view, &row);
        }
    })?;
    Ok(true)
}

/// Apply `event` and report it when it changed the tree: a disclosure
/// press, a step of the arrow keys.
pub(crate) fn apply_tree_event(
    tree: &mut TreeView,
    view: &mut crate::ViewContext<'_, TreeView>,
    event: TreeViewEvent<Arc<str>>,
) -> bool {
    if tree.apply_event(event.clone()) {
        view.emit(event);
        true
    } else {
        false
    }
}

/// Select `id` alone, as a plain click does, and report what changed.
fn select_alone(tree: &mut TreeView, view: &mut crate::ViewContext<'_, TreeView>, id: &Arc<str>) {
    let before = tree.selected_ids();
    if tree.select_only(id) {
        view.emit(TreeViewEvent::Select(Arc::clone(id)));
    }
    if tree.selected_ids() != before {
        view.emit(tree.selection_event());
    }
}

/// What a drag from `row` carries: the selection when `row` is in it.
fn dragged_rows(tree: &TreeView, row: &Arc<str>) -> Vec<Arc<str>> {
    if is_selected(&tree.nodes, row) {
        tree.selected_ids()
    } else {
        vec![Arc::clone(row)]
    }
}

/// Flip `id`'s toggle in `column`, as a press on it does, and report it;
/// the value it was set to.
fn flip_toggle(
    tree: &mut TreeView,
    view: &mut crate::ViewContext<'_, TreeView>,
    id: &Arc<str>,
    column: usize,
) -> bool {
    let on = !find_node(&tree.nodes, id)
        .and_then(|node| node.toggles.get(column).copied())
        .unwrap_or(false);
    if let Some(event) = set_toggle(tree, id, column, on) {
        view.emit(event);
    }
    on
}

/// Set a row's toggle; the event when it changed.
fn set_toggle(
    tree: &mut TreeView,
    id: &Arc<str>,
    column: usize,
    on: bool,
) -> Option<TreeToggleChanged> {
    let node = find_node_mut(&mut tree.nodes, id)?;
    // a drag across the column passes over rows without one
    if node.toggle_hidden(column) {
        return None;
    }
    if node.toggles.len() <= column {
        node.toggles.resize(column + 1, false);
    }
    if node.toggles[column] == on {
        return None;
    }
    node.toggles[column] = on;
    Some(TreeToggleChanged {
        id: Arc::clone(id),
        column,
        on,
    })
}

impl AppContext {
    /// An assistive-technology action on the virtual node of visible row
    /// `slot` of `tree`. Click selects the row as a click on it does and
    /// Focus moves the active row there as the arrow keys do, both with the
    /// tree taking the focus; Expand and Collapse open or close a branch as
    /// its disclosure does. A row that is gone or disabled, and an action
    /// the row cannot take, are refused.
    pub(crate) fn apply_tree_item_action(
        &mut self,
        document: DocumentId,
        tree: StableNodeId,
        slot: u16,
        action: &crate::AccessibilityAction,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.view_entity::<TreeView>(tree) else {
            return Ok(false);
        };
        let Some(row) = self.world().virtual_tree_row(tree, slot) else {
            return Ok(false);
        };
        if row.disabled {
            return Ok(false);
        }
        let id = row.id;
        match action {
            crate::AccessibilityAction::Click => {
                self.focus_node(document, tree)?;
                self.update_component(entity, |tree, view| select_alone(tree, view, &id))?;
                Ok(true)
            }
            crate::AccessibilityAction::Focus => {
                let focused = self.focus_node(document, tree)?;
                let moved = self.update_component(entity, |tree, view| {
                    apply_tree_event(tree, view, TreeViewEvent::Select(id))
                })?;
                Ok(focused || moved)
            }
            crate::AccessibilityAction::Expand | crate::AccessibilityAction::Collapse => {
                let expand = matches!(action, crate::AccessibilityAction::Expand);
                if !row.branch || row.expanded == expand {
                    return Ok(false);
                }
                self.update_component(entity, |tree, view| {
                    apply_tree_event(tree, view, TreeViewEvent::Toggle(id))
                })
            }
            _ => Ok(false),
        }
    }

    /// An assistive-technology action on the virtual node of the toggle in
    /// `column` of visible row `slot` of `tree` (#353). Click flips it as a
    /// press on it does, leaving the selection and the focus alone. A row
    /// that is gone or disabled, a column the tree does not have, and any
    /// other action are refused.
    pub(crate) fn apply_tree_toggle_action(
        &mut self,
        tree: StableNodeId,
        slot: u16,
        column: usize,
        action: &crate::AccessibilityAction,
    ) -> Result<bool, FrameworkError> {
        if !matches!(action, crate::AccessibilityAction::Click) {
            return Ok(false);
        }
        let Some(entity) = self.view_entity::<TreeView>(tree) else {
            return Ok(false);
        };
        let Some(row) = self.world().virtual_tree_toggle_row(tree, slot, column) else {
            return Ok(false);
        };
        if row.disabled {
            return Ok(false);
        }
        self.update_component(entity, |tree, view| {
            if column >= tree.columns.len() || find_node(&tree.nodes, &row.id).is_none() {
                return false;
            }
            flip_toggle(tree, view, &row.id, column);
            true
        })
    }

    /// Rename `id` in place: an input over its label takes the focus with
    /// the label selected.
    pub fn begin_tree_rename(
        &mut self,
        tree: Entity<TreeView>,
        id: &Arc<str>,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(tree.stable_id())?;
        let started = self.update_component(tree, |tree, _| {
            if !tree.renamable || find_node(&tree.nodes, id).is_none_or(|node| node.disabled) {
                return false;
            }
            tree.renaming = Some(Arc::clone(id));
            tree.rename_text = find_node(&tree.nodes, id)
                .map(|node| node.label.clone())
                .unwrap_or_default();
            true
        })?;
        let Some(input) = self
            .read(tree, |tree| tree.rename_input)?
            .filter(|_| started)
        else {
            return Ok(false);
        };
        self.focus_node(document, input)?;
        self.select_all_focused_text(document)?;
        Ok(true)
    }

    /// End a rename; `commit` reports the text.
    pub fn end_tree_rename(
        &mut self,
        tree: Entity<TreeView>,
        commit: bool,
    ) -> Result<bool, FrameworkError> {
        let Some((id, input)) =
            self.read(tree, |tree| tree.renaming.clone().zip(tree.rename_input))?
        else {
            return Ok(false);
        };
        let text = self.read(Entity::<TextInput>::from_stable_id(input), |input| {
            input.state.value.as_str().to_owned()
        })?;
        let document = self.document_of_node(tree.stable_id())?;
        let focused = self.world().focused(document) == Some(input);
        self.update_component(tree, |tree, view| {
            tree.renaming = None;
            if commit {
                view.emit(TreeRenameRequested { id, text });
            }
        })?;
        if focused {
            self.focus_node_in_place(document, tree.stable_id())?;
        }
        Ok(true)
    }

    /// Enter (`commit`) or Escape on a rename input ends the rename.
    pub(crate) fn end_focused_tree_rename(
        &mut self,
        document: DocumentId,
        commit: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(tree) = self
            .world()
            .focused(document)
            .and_then(|input| self.world().parent_id(input))
            .and_then(|parent| self.view_entity::<TreeView>(parent))
        else {
            return Ok(false);
        };
        self.end_tree_rename(tree, commit)
    }

    /// F2 on a focused tree, or Shift with an arrow: rename the active row,
    /// or extend the selection.
    pub(crate) fn focused_tree_key(
        &mut self,
        document: DocumentId,
        key: &str,
        shift: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(tree) = self
            .world()
            .focused(document)
            .and_then(|id| self.view_entity::<TreeView>(id))
        else {
            return Ok(false);
        };
        if key == "F2" && !shift {
            let Some(active) = self.read(tree, |tree| tree.active_id().cloned())? else {
                return Ok(false);
            };
            return self.begin_tree_rename(tree, &active);
        }
        let navigation = match key {
            "ArrowUp" => TreeNavigation::Previous,
            "ArrowDown" => TreeNavigation::Next,
            "Home" => TreeNavigation::First,
            "End" => TreeNavigation::Last,
            _ => return Ok(false),
        };
        if !shift {
            return Ok(false);
        }
        self.update_component(tree, |tree, view| {
            if !tree.multiple {
                return false;
            }
            let before = tree.selected_ids();
            tree.extend(navigation);
            if tree.selected_ids() != before {
                view.emit(tree.selection_event());
            }
            true
        })
    }

    /// The input a rename shows, over the row's label; gone when no rename
    /// is open.
    pub fn assemble_tree_rename(&mut self, tree: Entity<TreeView>) -> Result<bool, FrameworkError> {
        let (renaming, input) =
            self.read(tree, |tree| (tree.renaming.clone(), tree.rename_input))?;
        let input = input.filter(|input| self.world().contains(*input));
        let Some(id) = renaming else {
            if let Some(input) = input {
                let mut mutations = MutationQueue::new();
                mutations.despawn_subtree(input);
                self.commit_mutations(mutations)?;
                self.update_component(tree, |tree, _| tree.rename_input = None)?;
            }
            return Ok(false);
        };
        let label = self
            .read(tree, |tree| {
                find_node(&tree.nodes, &id).map(|node| node.label.clone())
            })?
            .unwrap_or_default();
        // the label's box, in the tree's own px
        let bounds = match (
            self.world().component_geometry(tree.stable_id()),
            self.world().component_layout_box(tree.stable_id()),
        ) {
            (Some(crate::ComponentGeometry::TreeView { rows, .. }), Some(origin)) => {
                rows.iter().find(|row| row.id == id).map(|row| LayoutBox {
                    x: row.label.bounds.x - origin.x,
                    y: row.bounds.y - origin.y,
                    width: row.label.bounds.width,
                    height: row.bounds.height,
                })
            }
            _ => None,
        };
        let created = input.is_none();
        let input = match input {
            Some(input) => input,
            None => {
                let document = self.document_of_node(tree.stable_id())?;
                let input = self.create_detached_component(
                    document,
                    TextInput::new(label).size(ControlSize::Small),
                )?;
                self.observe(input, tree, |tree, event: &TextChanged, _| {
                    tree.rename_text = event.value.as_str().to_owned();
                })?;
                self.on(tree, |tree, event: &FocusWithinChanged, cx| {
                    // leaving the tree commits
                    if !event.focused
                        && let Some(id) = tree.renaming.take()
                    {
                        let text = tree.rename_text.clone();
                        cx.emit(TreeRenameRequested { id, text });
                    }
                })?;
                self.update_component(tree, |tree, _| tree.rename_input = Some(input.stable_id()))?;
                input.stable_id()
            }
        };
        if let Some(bounds) = bounds {
            self.update_component(Entity::<TextInput>::from_stable_id(input), |input, _| {
                let layout = Arc::make_mut(&mut input.style.layout);
                layout.position = PositionSpec::Absolute;
                layout.offset_left = Some(LengthSpec::Px(bounds.x));
                layout.offset_top = Some(LengthSpec::Px(bounds.y));
                layout.width = Some(LengthSpec::Px(bounds.width.max(48.0)));
                layout.height = Some(LengthSpec::Px(bounds.height));
                layout.z_index = Some(1);
            })?;
        }
        self.append_children(tree.stable_id(), &[input])?;
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> TreeView {
        TreeView::new([
            TreeNode::branch(
                Arc::from("src"),
                "src",
                true,
                [
                    TreeNode::leaf(Arc::from("lib"), "lib.rs"),
                    TreeNode::leaf(Arc::from("main"), "main.rs"),
                ],
            )
            .selected(true),
            TreeNode::leaf(Arc::from("readme"), "README.md"),
        ])
    }

    #[test]
    fn visible_rows_follow_expansion() {
        let mut tree = sample();
        assert_eq!(tree.visible_rows().len(), 4);
        assert!(tree.apply_event(TreeViewEvent::Toggle(Arc::from("src"))));
        assert_eq!(tree.visible_rows().len(), 2);
        let event = tree.navigate(TreeNavigation::Next).expect("next");
        assert_eq!(event, TreeViewEvent::Select(Arc::from("readme")));
        assert_eq!(tree.selected_id().map(Arc::as_ref), Some("readme"));
    }

    /// The tree's height and its rows' boxes and label starts, as laid out.
    fn rows(theme: Option<nana_ui_core::ThemeMetrics>) -> (f32, Vec<LayoutBox>, Vec<f32>) {
        let mut context = AppContext::new();
        if let Some(metrics) = theme {
            context
                .set_theme_definition(
                    &nana_ui_core::ThemeDefinition::NANA_DARK.with_metrics(metrics),
                )
                .unwrap();
        }
        let tree = context.create_component(document(), sample()).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(300.0, 300.0))
            .unwrap();
        let height = context.world().layout_box(tree.stable_id()).unwrap().height;
        let Some(crate::ComponentGeometry::TreeView { rows, .. }) =
            context.world().component_geometry(tree.stable_id())
        else {
            panic!("tree geometry");
        };
        (
            height,
            rows.iter().map(|row| row.bounds).collect(),
            rows.iter().map(|row| row.label.bounds.x).collect(),
        )
    }

    #[test]
    fn rows_follow_the_installed_tree_metrics() {
        // the default theme: 28 px rows 2 px apart, 12 px per level
        let (height, bounds, labels) = rows(None);
        assert_eq!(height, 4.0 * 28.0 + 3.0 * 2.0);
        assert_eq!(bounds[1].y - bounds[0].y, 30.0);
        // `lib.rs` is a level in and has no disclosure; `src` has one
        assert_eq!(labels[1] - labels[0], 12.0 - nana_ui_core::type_scale::LINE);
        // the compact example: 20 px rows touching, 10 px per level
        let compact = nana_ui_core::COMPACT_METRICS;
        let (height, bounds, labels) = rows(Some(compact));
        assert_eq!(height, 4.0 * 20.0);
        assert_eq!(bounds[1].y - bounds[0].y, 20.0);
        assert_eq!(bounds[1].height, 20.0);
        assert_eq!(
            labels[1] - labels[0],
            compact.tree.indent - compact.tree.disclosure
        );
    }

    type Heard = Arc<std::sync::Mutex<Vec<String>>>;

    /// A laid-out tree, and every selection, disclosure and toggle event it
    /// reports.
    fn heard_tree(tree: TreeView) -> (AppContext, Entity<TreeView>, Heard) {
        let mut context = AppContext::new();
        let entity = context.create_component(document(), tree).unwrap();
        let heard = Heard::default();
        let events = Arc::clone(&heard);
        context
            .on(entity, move |_, event: &TreeViewEvent<Arc<str>>, _| {
                events.lock().unwrap().push(format!("{event:?}"));
            })
            .unwrap();
        let events = Arc::clone(&heard);
        context
            .on(entity, move |_, event: &TreeSelectionChanged, _| {
                events.lock().unwrap().push(format!("{event:?}"));
            })
            .unwrap();
        let events = Arc::clone(&heard);
        context
            .on(entity, move |_, event: &TreeToggleChanged, _| {
                events.lock().unwrap().push(format!("{event:?}"));
            })
            .unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(300.0, 300.0))
            .unwrap();
        context.rebuild_hit_test(document());
        (context, entity, heard)
    }

    fn take(heard: &Heard) -> Vec<String> {
        std::mem::take(&mut *heard.lock().unwrap())
    }

    fn row_node(tree: Entity<TreeView>, slot: u16) -> StableNodeId {
        crate::virtual_tree_item_id(tree.stable_id(), slot).unwrap()
    }

    fn act(
        context: &mut AppContext,
        target: StableNodeId,
        action: crate::AccessibilityAction,
    ) -> bool {
        context
            .apply_accessibility_action(
                document(),
                crate::AccessibilityActionRequest { target, action },
            )
            .unwrap()
    }

    /// The tree's own node and its rows' virtual nodes, in row order.
    fn semantics(
        context: &AppContext,
        tree: Entity<TreeView>,
    ) -> (crate::AccessibilityNode, Vec<crate::AccessibilityNode>) {
        let nodes = context.world().project_accessibility(document());
        let root = nodes
            .iter()
            .find(|node| node.id == tree.stable_id())
            .cloned()
            .expect("tree semantics");
        let rows = root
            .children
            .iter()
            .filter(|child| crate::decode_virtual_tree_item(**child).is_some())
            .map(|child| {
                nodes
                    .iter()
                    .find(|node| node.id == *child)
                    .cloned()
                    .expect("a row node for every row child")
            })
            .collect();
        (root, rows)
    }

    fn row_geometry(context: &AppContext, tree: Entity<TreeView>) -> Vec<TreeRowGeometry> {
        match context.world().component_geometry(tree.stable_id()) {
            Some(crate::ComponentGeometry::TreeView { rows, .. }) => rows,
            _ => panic!("tree geometry"),
        }
    }

    fn mid(bounds: LayoutBox) -> (f32, f32) {
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }

    fn pointer_click(context: &mut AppContext, (x, y): (f32, f32)) {
        use nana_ui_input::{InputPayload, PointerInput, PointerPhase};
        let mut input = crate::HeadlessInput::bind(context, document());
        for phase in [PointerPhase::Down, PointerPhase::Up] {
            input
                .route(
                    context,
                    InputPayload::Pointer(PointerInput::mouse(phase, x, y)),
                )
                .unwrap();
        }
    }

    #[test]
    fn rows_project_as_tree_items_with_level_and_state() {
        let (mut context, tree, _) = heard_tree(sample().label("文件"));
        let (root, rows) = semantics(&context, tree);
        assert_eq!(root.role, crate::AccessibilityRole::Tree);
        assert_eq!(root.label.as_deref(), Some("文件"));
        let labels = rows
            .iter()
            .map(|row| row.label.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["src", "lib.rs", "main.rs", "README.md"]);
        assert!(
            rows.iter()
                .all(|row| row.role == crate::AccessibilityRole::TreeItem
                    && row.parent == Some(tree.stable_id()))
        );
        let levels = rows.iter().map(|row| row.level).collect::<Vec<_>>();
        assert_eq!(levels, [Some(1), Some(2), Some(2), Some(1)]);
        let expanded = rows.iter().map(|row| row.expanded).collect::<Vec<_>>();
        assert_eq!(expanded, [Some(true), None, None, None]);
        let selected = rows.iter().map(|row| row.selected).collect::<Vec<_>>();
        assert_eq!(
            selected,
            [Some(true), Some(false), Some(false), Some(false)]
        );
        let bounds = row_geometry(&context, tree)
            .into_iter()
            .map(|row| row.bounds)
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter().map(|row| row.bounds).collect::<Vec<_>>(),
            bounds
        );
        assert!(rows.iter().all(|row| !row.focused), "the tree has no focus");

        // with the focus, the active row is the focused one
        context.take_system_work();
        context.focus_node(document(), tree.stable_id()).unwrap();
        let (_, rows) = semantics(&context, tree);
        let focused = rows.iter().map(|row| row.focused).collect::<Vec<_>>();
        assert_eq!(focused, [true, false, false, false]);

        // a selection that moves nothing on screen still reaches the delta
        context.take_system_work();
        context
            .update_component(tree, |tree, _| tree.select_only(&Arc::from("main")))
            .unwrap();
        let work = context.take_system_work();
        let delta = context.world().project_accessibility_delta(&work);
        let main = delta
            .updated
            .iter()
            .find(|node| node.id == row_node(tree, 3))
            .expect("the newly active row is projected again");
        assert_eq!(main.selected, Some(true));
        assert!(main.focused);

        // collapsing the branch takes its rows out, in the next delta too
        context.take_system_work();
        context
            .update_component(tree, |tree, _| {
                tree.apply_event(TreeViewEvent::Toggle(Arc::from("src")))
            })
            .unwrap();
        let work = context.take_system_work();
        let delta = context.world().project_accessibility_delta(&work);
        let projected = delta
            .updated
            .iter()
            .find(|node| node.id == tree.stable_id())
            .expect("the tree is projected again when its rows change");
        let children = projected
            .children
            .iter()
            .copied()
            .filter(|child| crate::decode_virtual_tree_item(*child).is_some())
            .collect::<Vec<_>>();
        assert_eq!(children, [row_node(tree, 1), row_node(tree, 2)]);
        let readme = delta
            .updated
            .iter()
            .find(|node| node.id == row_node(tree, 2))
            .expect("the rows come with the tree");
        assert_eq!(readme.label.as_deref(), Some("README.md"));
        assert_eq!(readme.level, Some(1));
        let (_, rows) = semantics(&context, tree);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].expanded, Some(false));
    }

    #[test]
    fn a_tree_without_a_label_has_no_name() {
        let (context, tree, _) = heard_tree(sample());
        let (root, _) = semantics(&context, tree);
        assert_eq!(root.label, None, "the active row's id is no name");
    }

    #[test]
    fn click_on_a_row_node_reports_what_a_click_on_the_row_does() {
        let (mut pointer, pointer_tree, pointer_heard) = heard_tree(sample());
        let main = mid(row_geometry(&pointer, pointer_tree)[2].bounds);
        pointer_click(&mut pointer, main);
        let clicked = take(&pointer_heard);
        assert_eq!(clicked.len(), 2, "{clicked:?}");

        let (mut reader, reader_tree, reader_heard) = heard_tree(sample());
        assert!(act(
            &mut reader,
            row_node(reader_tree, 3),
            crate::AccessibilityAction::Click
        ));
        assert_eq!(take(&reader_heard), clicked);
        assert_eq!(
            reader.world().focused(document()),
            Some(reader_tree.stable_id()),
            "the tree takes the focus as a click gives it"
        );
        let (_, rows) = semantics(&reader, reader_tree);
        assert_eq!(rows[2].selected, Some(true));
        assert!(rows[2].focused);
    }

    #[test]
    fn focus_on_a_row_node_moves_the_active_row_as_the_arrow_keys_do() {
        use nana_ui_input::{
            InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey,
        };
        let (mut keyboard, keyboard_tree, keyboard_heard) = heard_tree(sample());
        keyboard
            .focus_node(document(), keyboard_tree.stable_id())
            .unwrap();
        let mut input = crate::HeadlessInput::bind(&mut keyboard, document());
        input
            .route(
                &mut keyboard,
                InputPayload::Key(KeyInput {
                    physical: PhysicalKey("ArrowDown".into()),
                    logical: LogicalKey("ArrowDown".into()),
                    state: KeyState::Pressed,
                    repeat: false,
                    modifiers: InputModifiers::empty(),
                }),
            )
            .unwrap();
        let stepped = take(&keyboard_heard);
        assert_eq!(stepped, [r#"Select("lib")"#]);

        let (mut reader, reader_tree, reader_heard) = heard_tree(sample());
        assert!(act(
            &mut reader,
            row_node(reader_tree, 2),
            crate::AccessibilityAction::Focus
        ));
        assert_eq!(take(&reader_heard), stepped);
        assert_eq!(
            reader.world().focused(document()),
            Some(reader_tree.stable_id())
        );
        assert_eq!(
            reader
                .read(reader_tree, |tree| tree.active_id().cloned())
                .unwrap(),
            Some(Arc::from("lib"))
        );
        let (_, rows) = semantics(&reader, reader_tree);
        let focused = rows.iter().map(|row| row.focused).collect::<Vec<_>>();
        assert_eq!(focused, [false, true, false, false]);
        // already there: nothing moves
        assert!(!act(
            &mut reader,
            row_node(reader_tree, 2),
            crate::AccessibilityAction::Focus
        ));
        assert!(take(&reader_heard).is_empty());
    }

    #[test]
    fn expand_and_collapse_on_a_row_node_toggle_as_its_disclosure_does() {
        let (mut pointer, pointer_tree, pointer_heard) = heard_tree(sample());
        let disclosure = row_geometry(&pointer, pointer_tree)[0]
            .disclosure
            .expect("a branch has a disclosure");
        pointer_click(&mut pointer, mid(disclosure));
        let toggled = take(&pointer_heard);
        assert_eq!(toggled, [r#"Toggle("src")"#]);

        let (mut reader, tree, heard) = heard_tree(sample());
        let src = row_node(tree, 1);
        assert!(!act(&mut reader, src, crate::AccessibilityAction::Expand));
        assert!(act(&mut reader, src, crate::AccessibilityAction::Collapse));
        assert_eq!(take(&heard), toggled);
        assert_eq!(semantics(&reader, tree).1.len(), 2);
        assert!(!act(&mut reader, src, crate::AccessibilityAction::Collapse));
        // README.md is the second row now, and a leaf
        assert!(!act(
            &mut reader,
            row_node(tree, 2),
            crate::AccessibilityAction::Expand
        ));
        assert!(act(&mut reader, src, crate::AccessibilityAction::Expand));
        assert_eq!(take(&heard), toggled);
        assert_eq!(semantics(&reader, tree).1.len(), 4);
    }

    #[test]
    fn row_node_actions_refuse_wrong_targets() {
        let (mut context, tree, heard) = heard_tree(TreeView::new([
            TreeNode::leaf(Arc::from("a"), "a"),
            TreeNode::leaf(Arc::from("b"), "b").disabled(true),
        ]));
        let click = || crate::AccessibilityAction::Click;
        // past the last row, a disabled row, another node's rows
        assert!(!act(&mut context, row_node(tree, 3), click()));
        assert!(!act(&mut context, row_node(tree, 2), click()));
        assert!(!act(
            &mut context,
            row_node(tree, 2),
            crate::AccessibilityAction::Focus
        ));
        let elsewhere = StableNodeId::new(tree.stable_id().get() + 1000).unwrap();
        assert!(!act(
            &mut context,
            crate::virtual_tree_item_id(elsewhere, 1).unwrap(),
            click()
        ));
        // an action a row cannot take, and a row of another document
        assert!(!act(
            &mut context,
            row_node(tree, 1),
            crate::AccessibilityAction::Increment
        ));
        assert!(
            !context
                .apply_accessibility_action(
                    DocumentId::new(2).unwrap(),
                    crate::AccessibilityActionRequest {
                        target: row_node(tree, 1),
                        action: click(),
                    },
                )
                .unwrap()
        );
        assert!(take(&heard).is_empty());
        assert!(act(&mut context, row_node(tree, 1), click()));
    }

    #[test]
    fn menu_rows_tree_rows_and_toggles_never_decode_as_each_other() {
        for owner in [1, 0x1234, 0x3f_ffff_ffff] {
            let owner = StableNodeId::new(owner).unwrap();
            for slot in [1, 0x8000, u16::MAX] {
                let menu = crate::virtual_menu_item_id(owner, slot).unwrap();
                let tree = crate::virtual_tree_item_id(owner, slot).unwrap();
                assert_ne!(menu, tree);
                assert_eq!(crate::decode_virtual_menu_item(menu), Some((owner, slot)));
                assert_eq!(crate::decode_virtual_tree_item(menu), None);
                assert_eq!(crate::decode_virtual_tree_toggle(menu), None);
                assert_eq!(crate::decode_virtual_tree_item(tree), Some((owner, slot)));
                assert_eq!(crate::decode_virtual_menu_item(tree), None);
                assert_eq!(crate::decode_virtual_tree_toggle(tree), None);
                let mut toggles = Vec::new();
                for column in [0, 1, 254] {
                    let toggle = crate::virtual_tree_toggle_id(owner, slot, column).unwrap();
                    assert_ne!(toggle, menu);
                    assert_ne!(toggle, tree);
                    assert!(!toggles.contains(&toggle));
                    toggles.push(toggle);
                    assert_eq!(
                        crate::decode_virtual_tree_toggle(toggle),
                        Some((owner, slot, column))
                    );
                    assert_eq!(crate::decode_virtual_tree_item(toggle), None);
                    assert_eq!(crate::decode_virtual_menu_item(toggle), None);
                }
                assert_eq!(crate::virtual_tree_toggle_id(owner, slot, 255), None);
                assert_eq!(crate::virtual_tree_toggle_id(owner, slot, usize::MAX), None);
            }
            assert_eq!(crate::virtual_tree_item_id(owner, 0), None);
            assert_eq!(crate::virtual_menu_item_id(owner, 0), None);
            assert_eq!(crate::virtual_tree_toggle_id(owner, 0, 0), None);
        }
        let too_large = StableNodeId::new(0x40_0000_0000).unwrap();
        assert_eq!(crate::virtual_menu_item_id(too_large, 1), None);
        assert_eq!(crate::virtual_tree_item_id(too_large, 1), None);
        assert_eq!(crate::virtual_tree_toggle_id(too_large, 1, 0), None);
        let retained = StableNodeId::new(42).unwrap();
        assert_eq!(crate::decode_virtual_menu_item(retained), None);
        assert_eq!(crate::decode_virtual_tree_item(retained), None);
        assert_eq!(crate::decode_virtual_tree_toggle(retained), None);
        // a column on a menu row is no id any encoder makes
        let forged = StableNodeId::new(0x8000_0000_0000_0000 | (1 << 54) | (1 << 16) | 1).unwrap();
        assert_eq!(crate::decode_virtual_menu_item(forged), None);
        assert_eq!(crate::decode_virtual_tree_item(forged), None);
        assert_eq!(crate::decode_virtual_tree_toggle(forged), None);
    }

    /// A tree whose rows end in a visible and a locked toggle: `scene`
    /// (selected) holds `model`, and `light` is disabled with only its
    /// first toggle written.
    fn toggled() -> TreeView {
        TreeView::new([
            TreeNode::branch(
                Arc::from("scene"),
                "scene",
                true,
                [TreeNode::leaf(Arc::from("model"), "model").toggles([false, true])],
            )
            .selected(true)
            .toggles([true, false]),
            TreeNode::leaf(Arc::from("light"), "light")
                .toggles([true])
                .disabled(true),
        ])
        .columns([
            TreeToggleColumn::new(Icon::Eye, "显示"),
            TreeToggleColumn::new(Icon::ShieldCheck, "锁定"),
        ])
    }

    fn toggle_node(tree: Entity<TreeView>, slot: u16, column: usize) -> StableNodeId {
        crate::virtual_tree_toggle_id(tree.stable_id(), slot, column).unwrap()
    }

    /// Each row's toggle nodes, in column order.
    fn toggle_semantics(
        context: &AppContext,
        tree: Entity<TreeView>,
    ) -> Vec<Vec<crate::AccessibilityNode>> {
        let nodes = context.world().project_accessibility(document());
        let (_, rows) = semantics(context, tree);
        rows.iter()
            .map(|row| {
                row.children
                    .iter()
                    .map(|child| {
                        nodes
                            .iter()
                            .find(|node| node.id == *child)
                            .cloned()
                            .expect("a toggle node for every toggle child")
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn toggles_project_as_checkboxes_under_their_row() {
        let (mut context, tree, _) = heard_tree(toggled());
        let (_, rows) = semantics(&context, tree);
        let toggles = toggle_semantics(&context, tree);
        assert_eq!(toggles.len(), 3);
        let geometry = row_geometry(&context, tree);
        for (index, row) in rows.iter().enumerate() {
            let slot = index as u16 + 1;
            assert_eq!(
                row.children,
                [toggle_node(tree, slot, 0), toggle_node(tree, slot, 1)]
            );
            for (column, toggle) in toggles[index].iter().enumerate() {
                assert_eq!(toggle.role, crate::AccessibilityRole::Checkbox);
                assert_eq!(toggle.parent, Some(row.id));
                assert!(toggle.children.is_empty());
                assert_eq!(toggle.bounds, geometry[index].toggles[column].1);
                assert!(!toggle.focused);
                assert_eq!(toggle.selected, None);
            }
        }
        let names = toggles[0]
            .iter()
            .map(|toggle| toggle.label.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(names, [Some("显示"), Some("锁定")]);
        let checked = toggles
            .iter()
            .map(|row| row.iter().map(|toggle| toggle.checked).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(
            checked,
            [
                [Some(true), Some(false)],
                [Some(false), Some(true)],
                // a toggle the node does not write is off
                [Some(true), Some(false)],
            ]
        );
        let disabled = toggles
            .iter()
            .map(|row| row.iter().all(|toggle| toggle.disabled))
            .collect::<Vec<_>>();
        assert_eq!(disabled, [false, false, true]);
        // the toggles sit inside their row, the first column first
        assert!(
            geometry[0]
                .bounds
                .contains(toggles[0][0].bounds.x + 1.0, toggles[0][0].bounds.y + 1.0)
        );
        assert!(toggles[0][0].bounds.x < toggles[0][1].bounds.x);

        // the application writing a toggle back reaches the delta, with
        // nothing else changed
        context.take_system_work();
        context
            .update_component(tree, |tree, _| tree.nodes[0].toggles = vec![false, false])
            .unwrap();
        let work = context.take_system_work();
        let delta = context.world().project_accessibility_delta(&work);
        let visible = delta
            .updated
            .iter()
            .find(|node| node.id == toggle_node(tree, 1, 0))
            .expect("the written toggle is projected again");
        assert_eq!(visible.checked, Some(false));
        assert_eq!(visible.label.as_deref(), Some("显示"));
        assert_eq!(visible.parent, Some(row_node(tree, 1)));

        // renaming a column renames every row's toggle
        context.take_system_work();
        context
            .update_component(tree, |tree, _| {
                tree.columns[1].label = Arc::from("锁住");
            })
            .unwrap();
        let work = context.take_system_work();
        let delta = context.world().project_accessibility_delta(&work);
        let locked = delta
            .updated
            .iter()
            .find(|node| node.id == toggle_node(tree, 2, 1))
            .expect("a renamed column is projected again");
        assert_eq!(locked.label.as_deref(), Some("锁住"));
    }

    #[test]
    fn a_hidden_toggle_is_not_drawn_pressed_dragged_over_or_announced() {
        let (mut context, tree, heard) = heard_tree(
            TreeView::new([
                TreeNode::branch(
                    Arc::from("group"),
                    "group",
                    true,
                    [TreeNode::leaf(Arc::from("a"), "a").toggles([false, false])],
                )
                .hide_toggles(),
                TreeNode::leaf(Arc::from("b"), "b")
                    .toggles([false, false])
                    .hide_toggle(1),
                TreeNode::leaf(Arc::from("c"), "c").toggles([false, false]),
            ])
            .columns([
                TreeToggleColumn::new(Icon::Eye, "显示"),
                TreeToggleColumn::new(Icon::ShieldCheck, "锁定"),
            ]),
        );
        let geometry = row_geometry(&context, tree);
        let drawn = |row: usize| {
            geometry[row]
                .toggles
                .iter()
                .map(|(_, bounds, _)| bounds.width > 0.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(drawn(0), [false, false]);
        assert_eq!(drawn(1), [true, true]);
        assert_eq!(drawn(2), [true, false]);

        // a press where b's lock would be toggles nothing
        let lock_x = mid(geometry[1].toggles[1].1).0;
        pointer_click(&mut context, (lock_x, mid(geometry[2].bounds).1));
        assert!(
            take(&heard)
                .iter()
                .all(|event| !event.starts_with("TreeToggleChanged"))
        );

        // dragging down the lock column from a to c passes over b
        {
            use nana_ui_input::{InputPayload, PointerInput, PointerPhase};
            let mut input = crate::HeadlessInput::bind(&mut context, document());
            for (phase, row) in [
                (PointerPhase::Down, 1),
                (PointerPhase::Move, 2),
                (PointerPhase::Move, 3),
                (PointerPhase::Up, 3),
            ] {
                let y = mid(geometry[row].bounds).1;
                input
                    .route(
                        &mut context,
                        InputPayload::Pointer(PointerInput::mouse(phase, lock_x, y)),
                    )
                    .unwrap();
            }
        }
        assert_eq!(
            take(&heard),
            [
                r#"TreeToggleChanged { id: "a", column: 1, on: true }"#,
                r#"TreeToggleChanged { id: "c", column: 1, on: true }"#,
            ]
        );
        assert_eq!(
            context
                .read(tree, |tree| tree.nodes[1].toggles.clone())
                .unwrap(),
            [false, false],
            "b keeps no lock"
        );

        let announced = toggle_semantics(&context, tree)
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>();
        assert_eq!(announced, [0, 2, 1, 2]);
    }

    #[test]
    fn a_toggle_column_without_a_name_has_none() {
        let (context, tree, _) = heard_tree(
            TreeView::new([TreeNode::leaf(Arc::from("a"), "a")])
                .columns([TreeToggleColumn::new(Icon::Eye, "")]),
        );
        let toggles = toggle_semantics(&context, tree);
        assert_eq!(toggles[0].len(), 1);
        assert_eq!(toggles[0][0].label, None);
        assert_eq!(toggles[0][0].checked, Some(false));
    }

    #[test]
    fn click_on_a_toggle_node_reports_what_a_press_on_the_toggle_does() {
        let (mut pointer, pointer_tree, pointer_heard) = heard_tree(toggled());
        let locked = row_geometry(&pointer, pointer_tree)[1].toggles[1].1;
        pointer_click(&mut pointer, mid(locked));
        let pressed = take(&pointer_heard);
        assert_eq!(
            pressed,
            [r#"TreeToggleChanged { id: "model", column: 1, on: false }"#]
        );

        let (mut reader, reader_tree, reader_heard) = heard_tree(toggled());
        assert!(act(
            &mut reader,
            toggle_node(reader_tree, 2, 1),
            crate::AccessibilityAction::Click
        ));
        assert_eq!(take(&reader_heard), pressed);
        let read = |context: &AppContext, tree: Entity<TreeView>| {
            context
                .read(tree, |tree| (tree.nodes.clone(), tree.selected_ids()))
                .unwrap()
        };
        assert_eq!(read(&reader, reader_tree), read(&pointer, pointer_tree));
        let (_, rows) = semantics(&reader, reader_tree);
        let selected = rows.iter().map(|row| row.selected).collect::<Vec<_>>();
        assert_eq!(selected, [Some(true), Some(false), Some(false)]);
        assert_eq!(
            reader.world().focused(document()),
            None,
            "the focus stays where it was"
        );
        assert_eq!(
            toggle_semantics(&reader, reader_tree)[1][1].checked,
            Some(false)
        );

        // and back on, as a second press does
        assert!(act(
            &mut reader,
            toggle_node(reader_tree, 2, 1),
            crate::AccessibilityAction::Click
        ));
        assert_eq!(
            take(&reader_heard),
            [r#"TreeToggleChanged { id: "model", column: 1, on: true }"#]
        );
        // a toggle the node does not write yet is set on
        assert!(act(
            &mut reader,
            toggle_node(reader_tree, 1, 1),
            crate::AccessibilityAction::Click
        ));
        assert_eq!(
            take(&reader_heard),
            [r#"TreeToggleChanged { id: "scene", column: 1, on: true }"#]
        );
    }

    #[test]
    fn toggle_node_actions_refuse_wrong_targets() {
        let (mut context, tree, heard) = heard_tree(toggled());
        let click = || crate::AccessibilityAction::Click;
        // a disabled row, past the last row, past the last column
        assert!(!act(&mut context, toggle_node(tree, 3, 0), click()));
        assert!(!act(&mut context, toggle_node(tree, 4, 0), click()));
        assert!(!act(&mut context, toggle_node(tree, 1, 2), click()));
        // another node's toggles, and a toggle of another document
        let elsewhere = StableNodeId::new(tree.stable_id().get() + 1000).unwrap();
        assert!(!act(
            &mut context,
            crate::virtual_tree_toggle_id(elsewhere, 1, 0).unwrap(),
            click()
        ));
        assert!(
            !context
                .apply_accessibility_action(
                    DocumentId::new(2).unwrap(),
                    crate::AccessibilityActionRequest {
                        target: toggle_node(tree, 1, 0),
                        action: click(),
                    },
                )
                .unwrap()
        );
        // actions a toggle cannot take
        for action in [
            crate::AccessibilityAction::Focus,
            crate::AccessibilityAction::Expand,
            crate::AccessibilityAction::Collapse,
            crate::AccessibilityAction::Increment,
        ] {
            assert!(!act(&mut context, toggle_node(tree, 1, 0), action));
        }
        // a row that is gone: collapsing `scene` leaves two rows
        assert!(act(
            &mut context,
            row_node(tree, 1),
            crate::AccessibilityAction::Collapse
        ));
        take(&heard);
        assert!(!act(&mut context, toggle_node(tree, 3, 0), click()));
        assert!(take(&heard).is_empty());
        assert_eq!(
            context
                .read(tree, |tree| tree.nodes[1].toggles.clone())
                .unwrap(),
            [true],
            "the disabled row's toggles are untouched"
        );
        assert!(act(&mut context, toggle_node(tree, 1, 0), click()));

        // a tree without columns has no toggles to click
        let (mut plain, plain_tree, plain_heard) = heard_tree(sample());
        assert!(!act(&mut plain, toggle_node(plain_tree, 1, 0), click()));
        assert!(take(&plain_heard).is_empty());
        let (_, rows) = semantics(&plain, plain_tree);
        assert!(rows.iter().all(|row| row.children.is_empty()));
    }

    #[test]
    fn tree_projects_a_single_retained_surface() {
        let mut context = AppContext::new();
        let tree = context.create_component(document(), sample()).unwrap();
        let id = tree.stable_id();
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::TreeView { ref rows, .. }) if rows.len() == 4
        ));
    }
}
