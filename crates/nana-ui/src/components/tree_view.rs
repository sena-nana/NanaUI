use std::rc::Rc;

use iced::Element;
use iced::keyboard;
use iced::widget::column;

use crate::components::ControlSize;
use crate::icons::{Icon, icon};
use crate::sidebar::{SidebarRow, SidebarRowState};
use crate::theme::ThemeTokens;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode<Id> {
    pub id: Id,
    pub label: String,
    pub icon: Option<Icon>,
    pub branch: bool,
    pub expanded: bool,
    pub selected: bool,
    pub disabled: bool,
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
            children: children.into_iter().collect(),
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
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

impl TreeNavigation {
    pub fn from_iced_key(key: &keyboard::Key) -> Option<Self> {
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Self::Previous),
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Self::Next),
            keyboard::Key::Named(keyboard::key::Named::Home) => Some(Self::First),
            keyboard::Key::Named(keyboard::key::Named::End) => Some(Self::Last),
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => Some(Self::Parent),
            keyboard::Key::Named(keyboard::key::Named::ArrowRight) => Some(Self::Child),
            keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Self::Activate),
            keyboard::Key::Named(keyboard::key::Named::Space) => Some(Self::Toggle),
            _ => None,
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

pub struct TreeView<'a, Id, Message> {
    nodes: Vec<TreeNode<Id>>,
    on_event: Rc<dyn Fn(TreeViewEvent<Id>) -> Message + 'a>,
    size: ControlSize,
    tokens: ThemeTokens,
}

impl<'a, Id, Message> TreeView<'a, Id, Message>
where
    Id: Clone + Eq + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        nodes: impl IntoIterator<Item = TreeNode<Id>>,
        on_event: impl Fn(TreeViewEvent<Id>) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            on_event: Rc::new(on_event),
            size: ControlSize::Small,
            tokens: theme.into(),
        }
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let mut content = column![].spacing(1);
        for node in self.nodes {
            content = content.push(render_node(node, 0, self.size, &self.on_event, self.tokens));
        }
        content.into()
    }
}

fn render_node<'a, Id, Message>(
    node: TreeNode<Id>,
    depth: u16,
    size: ControlSize,
    on_event: &Rc<dyn Fn(TreeViewEvent<Id>) -> Message + 'a>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Id: Clone + Eq + 'a,
    Message: Clone + 'a,
{
    let mut row = SidebarRow::new(node.label)
        .depth(depth)
        .size(size)
        .state(if node.disabled {
            SidebarRowState::Disabled
        } else if node.selected {
            SidebarRowState::Active
        } else {
            SidebarRowState::Idle
        })
        .on_select(on_event(TreeViewEvent::Select(node.id.clone())));
    if let Some(node_icon) = node.icon {
        row = row.leading(icon(node_icon, 12.0, tokens.colors.muted));
    }
    if node.branch {
        row = row.disclosure(
            node.expanded,
            on_event(TreeViewEvent::Toggle(node.id.clone())),
        );
    }
    let mut content = column![row.view(tokens)].spacing(1);
    if node.branch && node.expanded {
        for child in node.children {
            content = content.push(render_node(
                child,
                depth.saturating_add(1),
                size,
                on_event,
                tokens,
            ));
        }
    }
    content.into()
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
    fn horizontal_navigation_expands_collapses_and_enters_branches() {
        assert_eq!(
            tree_navigation_event(&tree(false), Some(&"src"), TreeNavigation::Child),
            Some(TreeViewEvent::Toggle("src"))
        );
        assert_eq!(
            tree_navigation_event(&tree(true), Some(&"src"), TreeNavigation::Child),
            Some(TreeViewEvent::Select("lib"))
        );
        assert_eq!(
            tree_navigation_event(&tree(true), Some(&"src"), TreeNavigation::Parent),
            Some(TreeViewEvent::Toggle("src"))
        );
    }
}
