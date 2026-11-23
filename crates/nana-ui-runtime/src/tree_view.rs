//! Flattened disclosure tree. Application owns node identities and expansion.
//!
//! Which rows are selected is the application's: a
//! [`nana_ui_core::SelectionState`] handed to [`TreeView::selection`], the
//! same contract a table, a list or a grid uses (#117). The tree draws and
//! announces it, and asks for changes with [`TreeSelectionRequested`]: a
//! press asks for [`SelectionGesture::Press`] with its modifiers (Ctrl or
//! Cmd toggles, Shift ranges from the anchor), the arrow keys for
//! [`SelectionGesture::Move`] (Shift extends, Ctrl moves without selecting),
//! Ctrl+Space for [`SelectionGesture::Toggle`] and Ctrl+A for
//! [`SelectionGesture::SelectAll`]. The application applies the request over
//! [`TreeOrder::visible`] and hands the result back. [`TreeView::columns`] puts on
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
    ControlSize, Icon, LengthSpec, PositionSpec, SelectionGesture, SelectionModifiers,
    SelectionState, TreeNavigation, TreeNode, TreeOrder, TreeViewEvent, tree_navigation_event,
    tree_preorder,
};

use crate::view_components::{TextChanged, TextInput, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentTextRegion, ComponentView,
    DocumentId, Entity, FocusWithinChanged, FrameworkError, InteractionState, LayoutBox,
    MutationQueue, NodeKind, NodePointer, NodePointerHooks, NodeStyle, StableNodeId,
    TextHorizontalAlignment, TextValue, UiWorld, WidgetGlyph, WidgetIcon, WidgetPaintCtx,
    WidgetPaintOp, WidgetPaintOutput, WidgetQuad, WidgetVisual,
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

/// The selection a [`TreeView`] shows: the application's, shared. A new
/// value is a new `Arc`; an unchanged one compares by pointer.
pub type TreeSelection = Arc<SelectionState<Arc<str>>>;

/// A press, a key or an assistive-technology action on the row `id` asked
/// for the selection to change. Apply it to the application's
/// [`SelectionState`] over [`TreeOrder::visible`] and give the tree the
/// result ([`TreeView::selection`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSelectionRequested {
    pub id: Arc<str>,
    pub gesture: SelectionGesture,
}

impl TreeSelectionRequested {
    /// Apply the request to `selection` over the rows `nodes` shows, as
    /// [`SelectionState::apply`] with [`TreeOrder::visible`]. Returns whether
    /// anything changed.
    pub fn apply_to(
        &self,
        selection: &mut SelectionState<Arc<str>>,
        nodes: &[TreeNode<Arc<str>>],
    ) -> bool {
        selection.apply(
            self.gesture,
            Arc::clone(&self.id),
            &TreeOrder::visible(nodes),
        )
    }
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

/// `rows` were dragged out of the tree and let go at window `(x, y)`, the
/// coordinates a pointer event carries. Another part of the application
/// may take them there, say a list it maps the point into with
/// [`crate::UiWorld::pointer_layout_position`].
#[derive(Debug, Clone, PartialEq)]
pub struct TreeDragReleased {
    pub rows: Vec<Arc<str>>,
    pub x: f32,
    pub y: f32,
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
    /// Which rows are selected and which is current: the application's.
    /// In [`nana_ui_core::SelectionMode::Multiple`] the tree is announced as
    /// multi-selectable.
    pub selection: TreeSelection,
    /// On / off icons at the end of each row; a node's states are its
    /// [`TreeNode::toggles`].
    pub columns: Vec<TreeToggleColumn>,
    /// Rows drag onto other rows.
    pub reorderable: bool,
    /// F2 or a double click renames a row in place.
    pub renamable: bool,
    /// The tree's name for assistive technology: `Layers`.
    pub label: Option<Arc<str>>,
    /// The row the keyboard last moved to, until the application's
    /// selection catches up with it (several keys in one frame).
    pub(crate) cursor: Option<Arc<str>>,
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
            selection: TreeSelection::default(),
            columns: Vec::new(),
            reorderable: false,
            renamable: false,
            label: None,
            cursor: None,
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

    /// The selection to show: the application's [`SelectionState`].
    pub fn selection(mut self, selection: impl Into<TreeSelection>) -> Self {
        self.selection = selection.into();
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
        collect_rows(&self.nodes, &self.selection, 0, &mut rows);
        rows
    }

    /// The row the keyboard is on: where it last moved, else the
    /// selection's current row.
    pub fn active_id(&self) -> Option<&Arc<str>> {
        self.cursor
            .as_ref()
            .filter(|id| find_node(&self.nodes, id).is_some())
            .or_else(|| {
                self.selection
                    .current()
                    .filter(|id| find_node(&self.nodes, id).is_some())
            })
    }

    /// The row being renamed.
    pub fn renaming(&self) -> Option<&Arc<str>> {
        self.renaming.as_ref()
    }

    /// Apply a navigation step: [`TreeViewEvent::Toggle`] opens or closes a
    /// branch, [`TreeViewEvent::Select`] moves the keyboard to a row (the
    /// selection is the application's). Returns whether anything moved.
    pub fn apply_event(&mut self, event: TreeViewEvent<Arc<str>>) -> bool {
        match event {
            TreeViewEvent::Toggle(id) => toggle_node(&mut self.nodes, &id),
            TreeViewEvent::Select(id) => {
                if find_node(&self.nodes, &id).is_none() {
                    return false;
                }
                let moved = self.active_id() != Some(&id);
                self.cursor = Some(id);
                moved
            }
        }
    }

    /// Step the keyboard by `navigation` from the active row.
    pub fn navigate(&mut self, navigation: TreeNavigation) -> Option<TreeViewEvent<Arc<str>>> {
        let from = self.active_id().cloned();
        let event = tree_navigation_event(&self.nodes, from.as_ref(), navigation)?;
        self.apply_event(event.clone());
        Some(event)
    }

    fn intrinsic_height(&self, metrics: nana_ui_core::ThemeMetrics) -> f32 {
        let count = self.visible_rows().len().max(1) as f32;
        count * self.size.height_in(metrics) + (count - 1.0) * metrics.tree.row_gap
    }
}

impl ComponentView for TreeView {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TREE_VIEW.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activate_at: Some(crate::AppContext::activate_tree_at),
        assembler: Some(AppContext::assemble_tree_rename),
        hooks: Some(|hooks| {
            hooks.tree = Some(&TREE_HOOKS);
            crate::framework::routes::tree_view(hooks);
        }),
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
        // a gesture, a rename and the keyboard row are the tree's own; once
        // the application moves its current row, the keyboard follows it
        if next.selection.current() == self.selection.current() {
            next.cursor = self.cursor.take();
        }
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

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_TREE_VIEW
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let rows = self.visible_rows();
        let visual = TreeViewVisual {
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
        if world.widget_visual_as::<TreeViewVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
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
        let content =
            self.intrinsic_height(world.node_theme(id).metrics()) + padding.top + padding.bottom;
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
                multiselectable: self.selection.mode() == nana_ui_core::SelectionMode::Multiple,
                ..AccessibilityState::default()
            },
        );
    }
}

// A tree's depth is its data's (a file system, a JSON document), so none of
// these walks recurses by it: the read-only ones go through
// `tree_preorder`, the others keep their open levels on the heap.

fn collect_rows(
    nodes: &[TreeNode<Arc<str>>],
    selection: &SelectionState<Arc<str>>,
    depth: u8,
    rows: &mut Vec<TreeRowData>,
) {
    for step in tree_preorder(nodes, |node| node.branch && node.expanded) {
        let node = step.node;
        rows.push(TreeRowData {
            id: Arc::clone(&node.id),
            label: Arc::from(node.label.as_str()),
            icon: node.icon,
            depth: depth.saturating_add(u8::try_from(step.depth).unwrap_or(u8::MAX)),
            branch: node.branch,
            expanded: node.expanded,
            selected: selection.is_selected(&node.id),
            disabled: node.disabled,
            toggles: node.toggles.clone(),
            hidden_toggles: node.hidden_toggles,
        });
    }
}

/// Every node, collapsed ones' children too.
fn every_node(nodes: &[TreeNode<Arc<str>>]) -> impl Iterator<Item = &TreeNode<Arc<str>>> {
    tree_preorder(nodes, |_| true).map(|step| step.node)
}

fn find_node<'a>(nodes: &'a [TreeNode<Arc<str>>], id: &Arc<str>) -> Option<&'a TreeNode<Arc<str>>> {
    every_node(nodes).find(|node| &node.id == id)
}

fn find_node_mut<'a>(
    nodes: &'a mut [TreeNode<Arc<str>>],
    id: &Arc<str>,
) -> Option<&'a mut TreeNode<Arc<str>>> {
    let mut open = vec![nodes.iter_mut()];
    while let Some(level) = open.last_mut() {
        let Some(node) = level.next() else {
            open.pop();
            continue;
        };
        if &node.id == id {
            return Some(node);
        }
        open.push(node.children.iter_mut());
    }
    None
}

/// `inner` is `outer` or one of its descendants.
fn is_within(nodes: &[TreeNode<Arc<str>>], outer: &Arc<str>, inner: &Arc<str>) -> bool {
    find_node(nodes, outer)
        .is_some_and(|node| &node.id == inner || find_node(&node.children, inner).is_some())
}

fn toggle_node(nodes: &mut [TreeNode<Arc<str>>], id: &Arc<str>) -> bool {
    match find_node_mut(nodes, id) {
        Some(node) if node.branch => {
            node.expanded = !node.expanded;
            true
        }
        _ => false,
    }
}

const TREE_TOGGLE_SLOTS: u64 = 1 << 32;

/// Resolved flattened rows and drop mark for a mounted [`TreeView`].
#[derive(Debug, Clone, PartialEq)]
pub struct TreeViewLayout {
    pub rows: Vec<TreeRowGeometry>,
    pub drop: Option<TreeDropMark>,
}

/// Widget visual projected by [`TreeView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeViewVisual {
    pub rows: Arc<[TreeRowData]>,
    pub size: ControlSize,
    /// The toggle columns' icons.
    pub columns: Arc<[Icon]>,
    /// The toggle columns' names: what assistive technology calls each button.
    pub column_labels: Arc<[Arc<str>]>,
    pub drop: Option<TreeDropIntent>,
    /// The active row: the one assistive technology is on while the tree has focus.
    pub active: Option<Arc<str>>,
}

impl WidgetVisual for TreeViewVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_TREE_VIEW)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn has_virtual_nodes(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let tree_recipe = ctx.theme.recipes().tree_view();
        let layout = tree_view_geometry(
            ctx.bounds,
            &self.rows,
            self.size,
            &self.columns,
            self.drop.as_ref(),
            &ctx.style_model.palette,
            ctx.style_model.metrics,
            ctx.theme,
        );
        let mut out = WidgetPaintOutput {
            owns_node_text: true,
            ..WidgetPaintOutput::new()
        };
        for (index, row) in layout.rows.iter().enumerate() {
            let index = u64::try_from(index).unwrap_or(u64::MAX);
            // toggle columns: a slot range of their own, eight per row
            for (column, (icon, icon_bounds, color)) in row.toggles.iter().enumerate() {
                // a column this row has no toggle in
                if icon_bounds.width <= 0.0 {
                    continue;
                }
                let column = u64::try_from(column).unwrap_or(7).min(7);
                out.push(
                    TREE_TOGGLE_SLOTS
                        .saturating_add(index.saturating_mul(8))
                        .saturating_add(column),
                    WidgetPaintOp::Icon(WidgetIcon {
                        bounds: *icon_bounds,
                        glyph: WidgetGlyph::Icon(*icon),
                        color: Some(*color),
                        paint_color: ctx.matching_text_paint(*color),
                        align_to_adjacent_text: false,
                    }),
                );
            }
            if let Some(background) = row.background {
                out.push(
                    10u64.saturating_add(index),
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect: row.bounds,
                        fill: Some(background),
                        fill_paint: ctx.matching_background_paint(background),
                        corner_radius: [ctx.chrome_radii.tree_row; 4],
                        ..WidgetQuad::default()
                    }),
                );
            }
            if let Some(disclosure) = row.disclosure {
                let region = ComponentTextRegion {
                    bounds: disclosure,
                    content: TextValue::from(if row.expanded { "▾" } else { "▸" }),
                    color: row.label.color,
                    font_size: row.label.font_size,
                    font_weight: None,
                };
                out.push(
                    40u64.saturating_add(index),
                    WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                        &region,
                        ctx,
                        TextHorizontalAlignment::Center,
                        false,
                    )),
                );
            }
            if let Some((icon, icon_bounds, color)) = row.icon {
                out.push(
                    80u64.saturating_add(index),
                    WidgetPaintOp::Icon(WidgetIcon {
                        bounds: icon_bounds,
                        glyph: WidgetGlyph::Icon(icon),
                        color: Some(color),
                        paint_color: ctx.matching_text_paint(color),
                        align_to_adjacent_text: false,
                    }),
                );
            }
            out.push(
                110u64.saturating_add(index),
                WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                    &row.label,
                    ctx,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }
        // where a dragged row would land: a line between rows, or the row it would go into outlined
        if let Some(mark) = layout.drop {
            out.push(
                TREE_TOGGLE_SLOTS - 1,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: mark.bounds,
                    fill: (!mark.inside).then_some(mark.color),
                    border: mark
                        .inside
                        .then_some((mark.color, tree_recipe.drop_outline_width)),
                    corner_radius: [ctx.chrome_radii.tree_row; 4],
                    ..WidgetQuad::default()
                }),
            );
        }
        out
    }
}

impl UiWorld {
    /// Resolves `id`'s flattened tree row and drop-indicator layout when mounted as a [`TreeView`].
    #[must_use]
    pub fn tree_view_layout(&self, id: StableNodeId) -> Option<TreeViewLayout> {
        let visual = self.widget_visual_as::<TreeViewVisual>(id)?;
        let bounds = self.component_layout_box(id)?;
        let style_model = self.node_style_model(id);
        let theme = self.node_theme(id);
        Some(tree_view_geometry(
            bounds,
            &visual.rows,
            visual.size,
            &visual.columns,
            visual.drop.as_ref(),
            &style_model.palette,
            style_model.metrics,
            theme,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tree_view_geometry(
    bounds: LayoutBox,
    rows: &[TreeRowData],
    size: ControlSize,
    columns: &[Icon],
    drop: Option<&TreeDropIntent>,
    palette: &nana_ui_core::SemanticPalette,
    metrics: nana_ui_core::ThemeMetrics,
    theme: &nana_ui_core::CompiledTheme,
) -> TreeViewLayout {
    let recipe = theme.recipes().tree_view();
    let opacity = theme.style_model().opacity;
    let color = |role| palette.get_in(role, opacity).as_rgba_array();
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
                    color(if row.disabled {
                        recipe.disabled_icon
                    } else {
                        recipe.icon
                    }),
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
                        color(if on {
                            recipe.toggle_on
                        } else {
                            recipe.toggle_off
                        }),
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
                    color: Some(color(if row.disabled {
                        recipe.disabled_foreground
                    } else {
                        recipe.foreground
                    })),
                    font_size: size.text_size(),
                    font_weight: row.selected.then_some(recipe.selected_weight),
                },
                selected: row.selected,
                disabled: row.disabled,
                expanded: row.expanded,
                background: row.selected.then(|| color(recipe.selected_background)),
                toggles,
            }
        })
        .collect();
    let drop = drop.and_then(|intent| {
        let row = geometry.iter().find(|row| row.id == intent.target)?;
        let line = recipe.drop_line_width;
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
            color: color(recipe.drop_indicator),
        })
    });
    TreeViewLayout {
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
    let TreeViewLayout { rows, .. } = cx.world().tree_view_layout(tree.stable_id())?;
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
    let modifiers = crate::selection_modifiers(pointer.modifiers);
    cx.update_component(tree, |tree, view| {
        // A plain press on a row of a several-row selection may start a
        // drag of all of them: it selects that row alone on release.
        let select_on_release = modifiers == SelectionModifiers::NONE
            && tree.reorderable
            && tree.selection.len() > 1
            && tree.selection.is_selected(&id);
        tree.cursor = Some(Arc::clone(&id));
        if !select_on_release {
            view.emit(TreeSelectionRequested {
                id: Arc::clone(&id),
                gesture: SelectionGesture::Press(modifiers),
            });
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
        cx.begin_tree_rename_focus(tree, &id, false)?;
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
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    // Let go outside the tree: where, in window coordinates.
    let outside = tree_rows(cx, tree, &pointer).and_then(|(_, (x, y))| {
        let bounds = cx.world().component_layout_box(tree.stable_id())?;
        if bounds.contains(x, y) {
            return None;
        }
        cx.world().layout_pointer_position(tree.stable_id(), x, y)
    });
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
            } else if let Some((x, y)) = outside {
                view.emit(TreeDragReleased {
                    rows: dragged_rows(tree, &row),
                    x,
                    y,
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

/// Ask for `gesture` on `id`, the keyboard moving there too.
pub(crate) fn request_tree_selection(
    tree: &mut TreeView,
    view: &mut crate::ViewContext<'_, TreeView>,
    id: &Arc<str>,
    gesture: SelectionGesture,
) {
    tree.cursor = Some(Arc::clone(id));
    view.emit(TreeSelectionRequested {
        id: Arc::clone(id),
        gesture,
    });
}

/// Ask for `id` alone, as a plain click does.
fn select_alone(tree: &mut TreeView, view: &mut crate::ViewContext<'_, TreeView>, id: &Arc<str>) {
    request_tree_selection(
        tree,
        view,
        id,
        SelectionGesture::Press(SelectionModifiers::NONE),
    );
}

/// Step the keyboard by `navigation` and ask for the selection to follow
/// with `modifiers`; a branch it opens or closes is reported as before.
pub(crate) fn navigate_tree(
    tree: &mut TreeView,
    view: &mut crate::ViewContext<'_, TreeView>,
    navigation: TreeNavigation,
    modifiers: SelectionModifiers,
) -> bool {
    match tree.navigate(navigation) {
        Some(TreeViewEvent::Select(id)) => {
            view.emit(TreeSelectionRequested {
                id,
                gesture: SelectionGesture::Move(modifiers),
            });
            true
        }
        Some(event @ TreeViewEvent::Toggle(_)) => {
            view.emit(event);
            true
        }
        None => false,
    }
}

/// What a drag from `row` carries: the selection, in tree order, when `row`
/// is in it.
fn dragged_rows(tree: &TreeView, row: &Arc<str>) -> Vec<Arc<str>> {
    if tree.selection.is_selected(row) {
        tree.selection.selected_in(&TreeOrder::all(&tree.nodes))
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
                    if tree.active_id() == Some(&id) {
                        return false;
                    }
                    request_tree_selection(
                        tree,
                        view,
                        &id,
                        SelectionGesture::Move(SelectionModifiers::NONE),
                    );
                    true
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
        self.begin_tree_rename_focus(tree, id, true)
    }

    fn begin_tree_rename_focus(
        &mut self,
        tree: Entity<TreeView>,
        id: &Arc<str>,
        reveal: bool,
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
        if reveal {
            self.focus_node(document, input)?;
        } else {
            self.focus_node_in_place(document, input)?;
        }
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

    /// A selection key on a focused tree: F2 renames the active row; Shift
    /// with ArrowUp / ArrowDown / Home / End extends from the anchor; Ctrl
    /// (Cmd) with ArrowUp / ArrowDown moves without selecting, with Space
    /// toggles the active row and with A selects all. `modifiers` says which
    /// the dispatcher saw.
    pub(crate) fn focused_tree_key(
        &mut self,
        document: DocumentId,
        key: &str,
        modifiers: SelectionModifiers,
    ) -> Result<bool, FrameworkError> {
        let Some(tree) = self
            .world()
            .focused(document)
            .and_then(|id| self.view_entity::<TreeView>(id))
        else {
            return Ok(false);
        };
        if key == "F2" && modifiers == SelectionModifiers::NONE {
            let Some(active) = self.read(tree, |tree| tree.active_id().cloned())? else {
                return Ok(false);
            };
            return self.begin_tree_rename(tree, &active);
        }
        let navigation = match key {
            "ArrowUp" => Some(TreeNavigation::Previous),
            "ArrowDown" => Some(TreeNavigation::Next),
            "Home" if modifiers.extend => Some(TreeNavigation::First),
            "End" if modifiers.extend => Some(TreeNavigation::Last),
            _ => None,
        };
        if let Some(navigation) = navigation {
            if modifiers == SelectionModifiers::NONE {
                return Ok(false);
            }
            return self.update_component(tree, |tree, view| {
                navigate_tree(tree, view, navigation, modifiers)
            });
        }
        if !modifiers.toggle || modifiers.extend {
            return Ok(false);
        }
        let gesture = match key {
            " " | "Space" => SelectionGesture::Toggle,
            _ if key.eq_ignore_ascii_case("a") => SelectionGesture::SelectAll,
            _ => return Ok(false),
        };
        self.update_component(tree, |tree, view| {
            let id = tree
                .active_id()
                .cloned()
                .or_else(|| tree.nodes.first().map(|node| Arc::clone(&node.id)));
            let Some(id) = id else {
                return false;
            };
            if gesture == SelectionGesture::Toggle
                && find_node(&tree.nodes, &id).is_none_or(|node| node.disabled)
            {
                return false;
            }
            request_tree_selection(tree, view, &id, gesture);
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
            self.world().tree_view_layout(tree.stable_id()),
            self.world().component_layout_box(tree.stable_id()),
        ) {
            (Some(TreeViewLayout { rows, .. }), Some(origin)) => {
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
        self.append_children(tree.stable_id(), &[input])?;
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
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;
    use nana_ui_core::SelectionMode;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// The application's selection: `ids` selected, the first current.
    fn selection(mode: SelectionMode, ids: &[&str]) -> TreeSelection {
        let mut selection = SelectionState::new(mode);
        selection.set_selection(ids.iter().map(|id| Arc::from(*id)));
        if let Some(first) = ids.first() {
            selection.set_current(Arc::from(*first));
        }
        Arc::new(selection)
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
            ),
            TreeNode::leaf(Arc::from("readme"), "README.md"),
        ])
        .selection(selection(SelectionMode::Single, &["src"]))
    }

    #[test]
    fn visible_rows_follow_expansion() {
        let mut tree = sample();
        assert_eq!(tree.visible_rows().len(), 4);
        assert!(tree.apply_event(TreeViewEvent::Toggle(Arc::from("src"))));
        assert_eq!(tree.visible_rows().len(), 2);
        let event = tree.navigate(TreeNavigation::Next).expect("next");
        assert_eq!(event, TreeViewEvent::Select(Arc::from("readme")));
        assert_eq!(tree.active_id().map(Arc::as_ref), Some("readme"));
        // the selection is the application's: the step only moved the keyboard
        assert!(tree.selection.is_selected(&Arc::from("src")));
    }

    /// A chain of `depth` expanded branches with a leaf at the bottom, built
    /// without recursion.
    fn chain(depth: usize) -> TreeNode<Arc<str>> {
        let mut node = TreeNode::leaf(Arc::from("leaf"), "leaf");
        for level in (0..depth).rev() {
            node = TreeNode::branch(Arc::from(format!("n{level}")), "branch", true, [node]);
        }
        node
    }

    #[test]
    fn data_deeper_than_the_stack_walks_clones_compares_and_drops() {
        // A JSON document or a file system can nest this deep; unguarded,
        // every walk, the clone, the comparison and the drop took one frame
        // per level and ran out of this 512 KiB stack.
        std::thread::Builder::new()
            .stack_size(512 << 10)
            .spawn(|| {
                let depth = 20_000;
                let mut tree = TreeView::new([chain(depth)])
                    .selection(selection(SelectionMode::Single, &["leaf"]));
                assert_eq!(tree.visible_rows().len(), depth + 1);
                assert!(tree.visible_rows()[depth].selected);
                assert_eq!(tree.active_id().map(Arc::as_ref), Some("leaf"));
                let copy = tree.clone();
                assert_eq!(copy, tree);
                assert!(tree.apply_event(TreeViewEvent::Select(Arc::from("n7"))));
                assert_ne!(copy, tree);
                assert!(tree.navigate(TreeNavigation::Next).is_some());
                let last = format!("n{}", depth - 1);
                assert!(tree.apply_event(TreeViewEvent::Toggle(Arc::from(last.as_str()))));
                assert_eq!(tree.visible_rows().len(), depth);
                drop(copy);
                drop(tree);
            })
            .unwrap()
            .join()
            .expect("a walk of the tree's data ran out of stack");
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
        let Some(TreeViewLayout { rows, .. }) = context.world().tree_view_layout(tree.stable_id())
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

    /// A selection request, short: `press+shift(main)`.
    fn describe(request: &TreeSelectionRequested) -> String {
        let (kind, modifiers) = match request.gesture {
            SelectionGesture::Press(modifiers) => ("press", modifiers),
            SelectionGesture::Move(modifiers) => ("move", modifiers),
            SelectionGesture::Toggle => ("toggle", SelectionModifiers::NONE),
            SelectionGesture::SelectAll => ("all", SelectionModifiers::NONE),
        };
        let ctrl = if modifiers.toggle { "+ctrl" } else { "" };
        let shift = if modifiers.extend { "+shift" } else { "" };
        format!("{kind}{ctrl}{shift}({})", request.id)
    }

    /// A laid-out tree, and every selection request, disclosure and toggle
    /// event it reports. Selection requests are applied as an application
    /// would: to its state, which it gives back to the tree.
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
            .on(entity, move |tree, event: &TreeSelectionRequested, _| {
                events.lock().unwrap().push(describe(event));
                let mut selection = SelectionState::clone(&tree.selection);
                if event.apply_to(&mut selection, &tree.nodes) {
                    tree.selection = Arc::new(selection);
                }
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
        context
            .world()
            .tree_view_layout(tree.stable_id())
            .expect("tree geometry")
            .rows
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
            .update_component(tree, |tree, _| {
                tree.selection = selection(SelectionMode::Single, &["main"]);
            })
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
    fn keys_ask_the_application_for_selection_changes() {
        use nana_ui_input::{
            InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey,
        };
        let multiple = sample().selection(selection(SelectionMode::Multiple, &["src"]));
        let (mut context, tree, heard) = heard_tree(multiple);
        let (root, _) = semantics(&context, tree);
        assert!(root.multiselectable, "a multiple selection is announced");
        context.focus_node(document(), tree.stable_id()).unwrap();
        let mut input = crate::HeadlessInput::bind(&mut context, document());
        let key = |name: &'static str, modifiers| {
            InputPayload::Key(KeyInput {
                physical: PhysicalKey(name.into()),
                logical: LogicalKey(name.into()),
                state: KeyState::Pressed,
                repeat: false,
                modifiers,
            })
        };
        let ctrl = InputModifiers {
            control: true,
            ..InputModifiers::empty()
        };
        let shift = InputModifiers {
            shift: true,
            ..InputModifiers::empty()
        };
        let selected = |context: &AppContext| {
            context
                .read(tree, |tree| {
                    tree.selection
                        .selected_in(&TreeOrder::all(&tree.nodes))
                        .iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap()
        };

        // Ctrl with an arrow moves the keyboard without selecting
        input.route(&mut context, key("ArrowDown", ctrl)).unwrap();
        assert_eq!(take(&heard), ["move+ctrl(lib)"]);
        assert_eq!(selected(&context), ["src"]);
        // Ctrl+Space toggles the row the keyboard is on
        input.route(&mut context, key(" ", ctrl)).unwrap();
        assert_eq!(take(&heard), ["toggle(lib)"]);
        assert_eq!(selected(&context), ["src", "lib"]);
        // Shift with End extends from the anchor
        input.route(&mut context, key("End", shift)).unwrap();
        assert_eq!(take(&heard), ["move+shift(readme)"]);
        assert_eq!(selected(&context), ["lib", "main", "readme"]);
        // Ctrl+A selects every visible row
        input.route(&mut context, key("a", ctrl)).unwrap();
        assert_eq!(take(&heard), ["all(readme)"]);
        assert_eq!(selected(&context), ["src", "lib", "main", "readme"]);
        // a plain arrow moves and selects that row alone
        input
            .route(&mut context, key("ArrowUp", InputModifiers::empty()))
            .unwrap();
        assert_eq!(take(&heard), ["move(main)"]);
        assert_eq!(selected(&context), ["main"]);

        // a single selection is not announced as multi-selectable
        let (context, tree, _) = heard_tree(sample());
        assert!(!semantics(&context, tree).0.multiselectable);
    }

    #[test]
    fn collapsing_a_branch_keeps_the_selection_inside_it() {
        let mut tree = sample().selection(selection(SelectionMode::Multiple, &["main", "readme"]));
        assert!(tree.apply_event(TreeViewEvent::Toggle(Arc::from("src"))));
        let mut state = SelectionState::clone(&tree.selection);
        // the application checks its selection against every node, so a
        // hidden row stays selected; the visible order would drop it
        assert!(!state.reconcile(&TreeOrder::all(&tree.nodes)));
        assert!(state.is_selected(&Arc::from("main")));
        tree.selection = Arc::new(state);
        assert!(tree.apply_event(TreeViewEvent::Toggle(Arc::from("src"))));
        let rows = tree.visible_rows();
        assert!(rows[2].selected && rows[3].selected);
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
        assert_eq!(clicked, ["press(main)"]);

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
        assert_eq!(stepped, ["move(lib)"]);

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
            .toggles([true, false]),
            TreeNode::leaf(Arc::from("light"), "light")
                .toggles([true])
                .disabled(true),
        ])
        .columns([
            TreeToggleColumn::new(Icon::Eye, "显示"),
            TreeToggleColumn::new(Icon::ShieldCheck, "锁定"),
        ])
        .selection(selection(SelectionMode::Single, &["scene"]))
    }

    fn toggle_node(tree: Entity<TreeView>, slot: u16, column: usize) -> StableNodeId {
        crate::virtual_tree_toggle_id(tree.stable_id(), slot, column).unwrap()
    }

    /// Each row's toggle nodes, in column order.
    fn toggle_semantics(
        context: &AppContext,
        tree: Entity<TreeView>,
    ) -> Vec<Vec<crate::AccessibilityNode>> {
        let nodes = context
            .world()
            .project_accessibility_in_viewport(document());
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
                .read(tree, |tree| (tree.nodes.clone(), tree.selection.clone()))
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
            context.world().widget_visual_as::<TreeViewVisual>(id),
            Some(TreeViewVisual { rows, .. }) if rows.len() == 4
        ));
    }
}
