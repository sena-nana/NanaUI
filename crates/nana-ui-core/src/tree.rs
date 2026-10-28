//! Backend-neutral tree navigation. Runtime consumes this walk.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode<Id> {
    pub id: Id,
    pub label: String,
    pub icon: Option<crate::Icon>,
    pub branch: bool,
    pub expanded: bool,
    pub selected: bool,
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
            selected: false,
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
            selected: false,
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

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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
    parent: Option<&'a Id>,
    visible: &mut Vec<VisibleTreeNode<'a, Id>>,
) {
    for node in nodes {
        visible.push(VisibleTreeNode { node, parent });
        if node.branch && node.expanded {
            collect_visible_nodes(&node.children, Some(&node.id), visible);
        }
    }
}

pub fn tree_navigation_event<Id: Clone + Eq>(
    nodes: &[TreeNode<Id>],
    selected: Option<&Id>,
    navigation: TreeNavigation,
) -> Option<TreeViewEvent<Id>> {
    let mut visible = Vec::new();
    collect_visible_nodes(nodes, None, &mut visible);
    if visible.is_empty() {
        return None;
    }
    let selected_index =
        selected.and_then(|selected| visible.iter().position(|entry| &entry.node.id == selected));
    let current_index = selected_index.unwrap_or(0);
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
}
