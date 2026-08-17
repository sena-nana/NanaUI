use std::rc::Rc;

use iced::Element;
use iced::keyboard;
use iced::widget::column;

use crate::components::ControlSize;
use crate::icons::icon;
use crate::sidebar::{SidebarRow, SidebarRowState};
use crate::theme::ThemeTokens;

pub use nana_ui_core::{TreeNavigation, TreeNode, TreeViewEvent, tree_navigation_event};

pub fn tree_navigation_from_iced_key(key: &keyboard::Key) -> Option<TreeNavigation> {
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(TreeNavigation::Previous),
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(TreeNavigation::Next),
        keyboard::Key::Named(keyboard::key::Named::Home) => Some(TreeNavigation::First),
        keyboard::Key::Named(keyboard::key::Named::End) => Some(TreeNavigation::Last),
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => Some(TreeNavigation::Parent),
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => Some(TreeNavigation::Child),
        keyboard::Key::Named(keyboard::key::Named::Enter) => Some(TreeNavigation::Activate),
        keyboard::Key::Named(keyboard::key::Named::Space) => Some(TreeNavigation::Toggle),
        _ => None,
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
