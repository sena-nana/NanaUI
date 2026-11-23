//! Backend-neutral tree navigation. Runtime consumes this walk.
//!
//! Nodes carry no selection: which rows are selected is the application's
//! [`crate::SelectionState`], over the order [`TreeOrder`] reads from the
//! nodes.
//!
//! A tree's depth is its data's, with no bound (a file system, a JSON
//! document), so nothing here recurses by it: walks keep their open levels
//! on the heap ([`tree_preorder`]), and a node clones, compares and drops
//! without one call per level. `Debug` alone is derived and nests.

#[derive(Debug)]
pub struct TreeNode<Id> {
    pub id: Id,
    pub label: String,
    pub icon: Option<crate::Icon>,
    pub branch: bool,
    pub expanded: bool,
    pub disabled: bool,
    /// The row's on / off states, one per toggle column of the tree
    /// (visible, locked). Missing entries read as off.
    pub toggles: Vec<bool>,
    /// Toggle columns this row has no toggle in, one bit per column: the
    /// cell is not drawn, pressed, dragged across or announced. A folder row
    /// in a layer tree, say, has no visibility of its own.
    pub hidden_toggles: u64,
    pub children: Vec<TreeNode<Id>>,
}

impl<Id> TreeNode<Id> {
    pub fn leaf(id: Id, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            icon: None,
            branch: false,
            expanded: false,
            disabled: false,
            toggles: Vec::new(),
            hidden_toggles: 0,
            children: Vec::new(),
        }
    }

    pub fn branch(
        id: Id,
        label: impl Into<String>,
        expanded: bool,
        children: impl IntoIterator<Item = TreeNode<Id>>,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            icon: None,
            branch: true,
            expanded,
            disabled: false,
            toggles: Vec::new(),
            hidden_toggles: 0,
            children: children.into_iter().collect(),
        }
    }

    pub fn icon(mut self, icon: crate::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The row's toggle states, one per toggle column.
    pub fn toggles(mut self, toggles: impl IntoIterator<Item = bool>) -> Self {
        self.toggles = toggles.into_iter().collect();
        self
    }

    /// Leave toggle `column` off this row: nothing drawn, pressed or
    /// announced there. Columns past 63 cannot be hidden.
    pub fn hide_toggle(mut self, column: usize) -> Self {
        if column < 64 {
            self.hidden_toggles |= 1 << column;
        }
        self
    }

    /// Leave every toggle column off this row.
    pub fn hide_toggles(mut self) -> Self {
        self.hidden_toggles = u64::MAX;
        self
    }

    /// Whether this row has no toggle in `column`.
    pub fn toggle_hidden(&self, column: usize) -> bool {
        toggle_hidden(self.hidden_toggles, column)
    }
}

/// Whether bit `column` of a [`TreeNode::hidden_toggles`] mask is set.
pub fn toggle_hidden(mask: u64, column: usize) -> bool {
    column < 64 && mask & (1 << column) != 0
}

impl<Id> TreeNode<Id> {
    /// This node without its children.
    fn shallow_clone(&self) -> Self
    where
        Id: Clone,
    {
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            icon: self.icon,
            branch: self.branch,
            expanded: self.expanded,
            disabled: self.disabled,
            toggles: self.toggles.clone(),
            hidden_toggles: self.hidden_toggles,
            children: Vec::with_capacity(self.children.len()),
        }
    }

    /// Everything but the children is equal.
    fn shallow_eq(&self, other: &Self) -> bool
    where
        Id: PartialEq,
    {
        self.id == other.id
            && self.label == other.label
            && self.icon == other.icon
            && self.branch == other.branch
            && self.expanded == other.expanded
            && self.disabled == other.disabled
            && self.toggles == other.toggles
            && self.hidden_toggles == other.hidden_toggles
            && self.children.len() == other.children.len()
    }
}

impl<Id: Clone> Clone for TreeNode<Id> {
    fn clone(&self) -> Self {
        // Each open level: the node being copied, its copy so far, and the
        // next child to copy into it.
        let mut open = vec![(self, self.shallow_clone(), 0)];
        loop {
            let (source, _, next) = open.last_mut().expect("the root is open");
            let source: &Self = source;
            if let Some(child) = source.children.get(*next) {
                *next += 1;
                open.push((child, child.shallow_clone(), 0));
                continue;
            }
            let (_, copy, _) = open.pop().expect("the root is open");
            match open.last_mut() {
                Some((_, parent, _)) => parent.children.push(copy),
                None => return copy,
            }
        }
    }
}

impl<Id: PartialEq> PartialEq for TreeNode<Id> {
    fn eq(&self, other: &Self) -> bool {
        let mut pairs = vec![(self, other)];
        while let Some((a, b)) = pairs.pop() {
            if !a.shallow_eq(b) {
                return false;
            }
            pairs.extend(a.children.iter().zip(&b.children));
        }
        true
    }
}

impl<Id: Eq> Eq for TreeNode<Id> {}

impl<Id> Drop for TreeNode<Id> {
    fn drop(&mut self) {
        // Every descendant leaves its parent before it drops, so each node
        // drops with no children and no drop runs inside another.
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
    }
}

/// One node of a [`tree_preorder`] walk.
pub struct TreeStep<'a, Id> {
    pub node: &'a TreeNode<Id>,
    pub parent: Option<&'a TreeNode<Id>>,
    /// 0 for the nodes of the slice the walk started from.
    pub depth: usize,
}

/// The nodes of `nodes` and their descendants in document order: a node,
/// then its children, then its next sibling. A node's children are entered
/// only when `descend` says so. The open levels are kept on the heap, so any
/// depth walks on any stack.
pub fn tree_preorder<'a, Id, F>(nodes: &'a [TreeNode<Id>], descend: F) -> TreePreorder<'a, Id, F>
where
    F: FnMut(&TreeNode<Id>) -> bool,
{
    TreePreorder {
        open: vec![(None, nodes.iter())],
        descend,
    }
}

/// One open level of a [`tree_preorder`] walk: the node whose children it
/// walks (`None` at the top), and the children left.
type OpenLevel<'a, Id> = (Option<&'a TreeNode<Id>>, std::slice::Iter<'a, TreeNode<Id>>);

/// See [`tree_preorder`].
pub struct TreePreorder<'a, Id, F> {
    open: Vec<OpenLevel<'a, Id>>,
    descend: F,
}

impl<'a, Id, F> Iterator for TreePreorder<'a, Id, F>
where
    F: FnMut(&TreeNode<Id>) -> bool,
{
    type Item = TreeStep<'a, Id>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (parent, level) = self.open.last_mut()?;
            let parent = *parent;
            let Some(node) = level.next() else {
                self.open.pop();
                continue;
            };
            let depth = self.open.len() - 1;
            if !node.children.is_empty() && (self.descend)(node) {
                self.open.push((Some(node), node.children.iter()));
            }
            return Some(TreeStep {
                node,
                parent,
                depth,
            });
        }
    }
}

/// A tree's rows in the order the user sees them, as a
/// [`crate::SelectionOrder`]: [`Self::visible`] (expanded branches only) is
/// what Shift ranges, the arrow keys and select-all run over;
/// [`Self::all`] (collapsed children too) is what
/// [`crate::SelectionState::reconcile`] checks against, so collapsing a
/// branch does not drop the selection inside it. Disabled rows cannot be
/// selected. Built once per use: one vector of references, no copies of the
/// nodes.
pub struct TreeOrder<'a, Id> {
    rows: Vec<&'a TreeNode<Id>>,
}

impl<'a, Id> TreeOrder<'a, Id> {
    /// The rows a user can see: children of collapsed branches left out.
    pub fn visible(nodes: &'a [TreeNode<Id>]) -> Self {
        Self {
            rows: tree_preorder(nodes, |node| node.branch && node.expanded)
                .map(|step| step.node)
                .collect(),
        }
    }

    /// Every node, collapsed branches' children too.
    pub fn all(nodes: &'a [TreeNode<Id>]) -> Self {
        Self {
            rows: tree_preorder(nodes, |_| true)
                .map(|step| step.node)
                .collect(),
        }
    }
}

impl<Id: Clone + PartialEq> crate::SelectionOrder<Id> for TreeOrder<'_, Id> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key_at(&self, index: usize) -> Option<Id> {
        self.rows.get(index).map(|node| node.id.clone())
    }

    fn index_of(&self, key: &Id) -> Option<usize> {
        self.rows.iter().position(|node| node.id == *key)
    }

    fn is_selectable(&self, index: usize) -> bool {
        self.rows.get(index).is_some_and(|node| !node.disabled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeViewEvent<Id> {
    Toggle(Id),
    Select(Id),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNavigation {
    Previous,
    Next,
    First,
    Last,
    Parent,
    Child,
    Activate,
    Toggle,
}

#[derive(Clone, Copy)]
struct VisibleTreeNode<'a, Id> {
    node: &'a TreeNode<Id>,
    parent: Option<&'a Id>,
}

fn collect_visible_nodes<'a, Id>(
    nodes: &'a [TreeNode<Id>],
    visible: &mut Vec<VisibleTreeNode<'a, Id>>,
) {
    visible.extend(
        tree_preorder(nodes, |node| node.branch && node.expanded).map(|step| VisibleTreeNode {
            node: step.node,
            parent: step.parent.map(|parent| &parent.id),
        }),
    );
}

/// Where `navigation` goes from the row `current` (the first row when
/// `None`): [`TreeViewEvent::Select`] names the row the keyboard moves to,
/// [`TreeViewEvent::Toggle`] the branch it opens or closes.
pub fn tree_navigation_event<Id: Clone + Eq>(
    nodes: &[TreeNode<Id>],
    current: Option<&Id>,
    navigation: TreeNavigation,
) -> Option<TreeViewEvent<Id>> {
    let mut visible = Vec::new();
    collect_visible_nodes(nodes, &mut visible);
    if visible.is_empty() {
        return None;
    }
    let current_index = current
        .and_then(|current| visible.iter().position(|entry| &entry.node.id == current))
        .unwrap_or(0);
    let current = &visible[current_index];
    match navigation {
        TreeNavigation::Previous => visible
            .get(current_index.saturating_sub(1))
            .map(|entry| TreeViewEvent::Select(entry.node.id.clone())),
        TreeNavigation::Next => visible
            .get((current_index + 1).min(visible.len() - 1))
            .map(|entry| TreeViewEvent::Select(entry.node.id.clone())),
        TreeNavigation::First => Some(TreeViewEvent::Select(visible[0].node.id.clone())),
        TreeNavigation::Last => Some(TreeViewEvent::Select(
            visible
                .last()
                .expect("visible tree is non-empty")
                .node
                .id
                .clone(),
        )),
        TreeNavigation::Parent if current.node.branch && current.node.expanded => {
            Some(TreeViewEvent::Toggle(current.node.id.clone()))
        }
        TreeNavigation::Parent => current
            .parent
            .map(|parent| TreeViewEvent::Select(parent.clone())),
        TreeNavigation::Child if current.node.branch && !current.node.expanded => {
            Some(TreeViewEvent::Toggle(current.node.id.clone()))
        }
        TreeNavigation::Child => current
            .node
            .children
            .first()
            .map(|child| TreeViewEvent::Select(child.id.clone())),
        TreeNavigation::Activate => Some(TreeViewEvent::Select(current.node.id.clone())),
        TreeNavigation::Toggle if current.node.branch => {
            Some(TreeViewEvent::Toggle(current.node.id.clone()))
        }
        TreeNavigation::Toggle => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(expanded: bool) -> Vec<TreeNode<&'static str>> {
        vec![
            TreeNode::branch(
                "src",
                "src",
                expanded,
                [
                    TreeNode::leaf("lib", "lib.rs"),
                    TreeNode::leaf("main", "main.rs"),
                ],
            ),
            TreeNode::leaf("readme", "README.md"),
        ]
    }

    #[test]
    fn navigation_only_walks_visible_nodes_and_preserves_stable_ids() {
        assert_eq!(
            tree_navigation_event(&tree(false), Some(&"src"), TreeNavigation::Next),
            Some(TreeViewEvent::Select("readme"))
        );
        assert_eq!(
            tree_navigation_event(&tree(true), Some(&"src"), TreeNavigation::Next),
            Some(TreeViewEvent::Select("lib"))
        );
        assert_eq!(
            tree_navigation_event(&tree(true), Some(&"main"), TreeNavigation::Parent),
            Some(TreeViewEvent::Select("src"))
        );
    }

    #[test]
    fn orders_walk_visible_or_every_row_and_skip_disabled_ones() {
        use crate::{SelectionGesture, SelectionModifiers, SelectionOrder, SelectionState};
        let mut nodes = tree(false);
        nodes[1].disabled = true;
        let visible = TreeOrder::visible(&nodes);
        let all = TreeOrder::all(&nodes);
        assert_eq!(
            (0..visible.len())
                .filter_map(|index| visible.key_at(index))
                .collect::<Vec<_>>(),
            ["src", "readme"]
        );
        assert_eq!(all.index_of(&"main"), Some(2));
        assert!(!all.is_selectable(3), "README.md is disabled");

        let mut selection = SelectionState::multiple();
        selection.set_selection(["lib", "main"]);
        // the collapsed branch's rows are still nodes: nothing is dropped
        assert!(!selection.reconcile(&all));
        assert!(selection.is_selected(&"main"));
        // a range over visible rows skips the hidden ones and the disabled one
        selection.apply(
            SelectionGesture::Press(SelectionModifiers::NONE),
            "src",
            &visible,
        );
        selection.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            "readme",
            &visible,
        );
        assert_eq!(selection.selected_in(&all), ["src"]);
    }
}
