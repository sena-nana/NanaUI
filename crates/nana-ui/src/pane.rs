use std::borrow::Cow;
use std::rc::Rc;

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::icons::{Icon, icon};
use crate::split_pane::SplitAxis;
use crate::theme::ThemeTokens;
use crate::widgets::{ButtonKind, button_style};

#[derive(Debug, Clone, PartialEq)]
pub enum PaneTreeNode<PaneId, SplitId> {
    Leaf {
        pane_id: PaneId,
    },
    Split {
        split_id: SplitId,
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneTreeNode<PaneId, SplitId>>,
        second: Box<PaneTreeNode<PaneId, SplitId>>,
    },
}

impl<PaneId, SplitId> PaneTreeNode<PaneId, SplitId> {
    pub fn leaf(pane_id: PaneId) -> Self {
        Self::Leaf { pane_id }
    }

    pub fn split(
        split_id: SplitId,
        axis: SplitAxis,
        ratio: f32,
        first: Self,
        second: Self,
    ) -> Self {
        Self::Split {
            split_id,
            axis,
            ratio: ratio.clamp(0.05, 0.95),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    pub fn visit_leaves(&self, mut visitor: impl FnMut(&PaneId)) {
        self.visit_leaves_with(&mut visitor);
    }

    fn visit_leaves_with(&self, visitor: &mut impl FnMut(&PaneId)) {
        match self {
            Self::Leaf { pane_id } => visitor(pane_id),
            Self::Split { first, second, .. } => {
                first.visit_leaves_with(visitor);
                second.visit_leaves_with(visitor);
            }
        }
    }

    pub fn visit_splits(&self, mut visitor: impl FnMut(&SplitId, SplitAxis, f32)) {
        self.visit_splits_with(&mut visitor);
    }

    fn visit_splits_with(&self, visitor: &mut impl FnMut(&SplitId, SplitAxis, f32)) {
        if let Self::Split {
            split_id,
            axis,
            ratio,
            first,
            second,
        } = self
        {
            visitor(split_id, *axis, *ratio);
            first.visit_splits_with(visitor);
            second.visit_splits_with(visitor);
        }
    }
}

type LeafRenderer<'a, PaneId, Message> = Rc<dyn Fn(&PaneId) -> Element<'a, Message> + 'a>;
type SplitRenderer<'a, SplitId, Message> = Rc<
    dyn Fn(
            &SplitId,
            SplitAxis,
            f32,
            Element<'a, Message>,
            Element<'a, Message>,
        ) -> Element<'a, Message>
        + 'a,
>;

pub struct PaneTree<'a, PaneId, SplitId, Message> {
    root: PaneTreeNode<PaneId, SplitId>,
    render_leaf: LeafRenderer<'a, PaneId, Message>,
    render_split: SplitRenderer<'a, SplitId, Message>,
}

impl<'a, PaneId, SplitId, Message> PaneTree<'a, PaneId, SplitId, Message>
where
    PaneId: 'a,
    SplitId: 'a,
    Message: 'a,
{
    pub fn new(
        root: PaneTreeNode<PaneId, SplitId>,
        render_leaf: impl Fn(&PaneId) -> Element<'a, Message> + 'a,
        render_split: impl Fn(
            &SplitId,
            SplitAxis,
            f32,
            Element<'a, Message>,
            Element<'a, Message>,
        ) -> Element<'a, Message>
        + 'a,
    ) -> Self {
        Self {
            root,
            render_leaf: Rc::new(render_leaf),
            render_split: Rc::new(render_split),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        render_node(self.root, &self.render_leaf, &self.render_split)
    }
}

fn render_node<'a, PaneId, SplitId, Message>(
    node: PaneTreeNode<PaneId, SplitId>,
    render_leaf: &LeafRenderer<'a, PaneId, Message>,
    render_split: &SplitRenderer<'a, SplitId, Message>,
) -> Element<'a, Message>
where
    PaneId: 'a,
    SplitId: 'a,
    Message: 'a,
{
    match node {
        PaneTreeNode::Leaf { pane_id } => render_leaf(&pane_id),
        PaneTreeNode::Split {
            split_id,
            axis,
            ratio,
            first,
            second,
        } => {
            let first = render_node(*first, render_leaf, render_split);
            let second = render_node(*second, render_leaf, render_split);
            render_split(&split_id, axis, ratio, first, second)
        }
    }
}

pub fn ratio_pane_split<'a, Message: 'a>(
    axis: SplitAxis,
    ratio: f32,
    first: Element<'a, Message>,
    second: Element<'a, Message>,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message> {
    let (first_portion, second_portion) = split_ratio_portions(ratio);
    let colors = theme.into().colors;
    match axis {
        SplitAxis::Horizontal => row![
            container(first)
                .width(Length::FillPortion(first_portion))
                .height(Length::Fill),
            container(iced::widget::space())
                .width(Length::Fixed(1.0))
                .height(Length::Fill)
                .style(move |_| {
                    iced::widget::container::Style::default().background(colors.border)
                }),
            container(second)
                .width(Length::FillPortion(second_portion))
                .height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        SplitAxis::Vertical => column![
            container(first)
                .width(Length::Fill)
                .height(Length::FillPortion(first_portion)),
            container(iced::widget::space())
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(move |_| {
                    iced::widget::container::Style::default().background(colors.border)
                }),
            container(second)
                .width(Length::Fill)
                .height(Length::FillPortion(second_portion)),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
    }
}

fn split_ratio_portions(ratio: f32) -> (u16, u16) {
    let first = (ratio.clamp(0.05, 0.95) * 1_000.0).round() as u16;
    (first.max(1), 1_000_u16.saturating_sub(first).max(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneChromeActionKind {
    Focus,
    SplitHorizontal,
    SplitVertical,
    MoveToWindow,
    MoveToNextPane,
    ClosePane,
    CloseItem,
    Custom,
}

#[derive(Debug, Clone)]
pub struct PaneChromeAction<'a, Message> {
    pub kind: PaneChromeActionKind,
    pub label: Cow<'a, str>,
    pub icon: Option<Icon>,
    pub message: Message,
}

impl<'a, Message> PaneChromeAction<'a, Message> {
    pub fn new(
        kind: PaneChromeActionKind,
        label: impl Into<Cow<'a, str>>,
        message: Message,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            icon: None,
            message,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

pub struct PaneChrome<'a, Message> {
    tabs: Element<'a, Message>,
    body: Element<'a, Message>,
    actions: Vec<PaneChromeAction<'a, Message>>,
    active: bool,
    tokens: ThemeTokens,
}

impl<'a, Message> PaneChrome<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        tabs: impl Into<Element<'a, Message>>,
        body: impl Into<Element<'a, Message>>,
        actions: impl IntoIterator<Item = PaneChromeAction<'a, Message>>,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            tabs: tabs.into(),
            body: body.into(),
            actions: actions.into_iter().collect(),
            active: true,
            tokens: theme.into(),
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let colors = self.tokens.colors;
        let mut chrome = row![container(self.tabs).width(Length::Fill)]
            .width(Length::Fill)
            .align_y(Alignment::Center);
        for action in self.actions {
            let content: Element<'a, Message> = action.icon.map_or_else(
                || text(action.label).size(10).into(),
                |action_icon| icon(action_icon, 13.0, colors.muted),
            );
            chrome = chrome.push(
                button(content)
                    .on_press(action.message)
                    .style(button_style(self.tokens, ButtonKind::Text)),
            );
        }
        let chrome = container(chrome)
            .width(Length::Fill)
            .height(Length::Fixed(34.0))
            .padding([0, 8])
            .style(move |_| {
                iced::widget::container::Style::default().background(if self.active {
                    colors.surface
                } else {
                    colors.faint
                })
            });
        column![chrome, self.body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_tree_preserves_leaf_and_split_identity_order() {
        let tree = PaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            0.6,
            PaneTreeNode::leaf("left"),
            PaneTreeNode::split(
                "right-stack",
                SplitAxis::Vertical,
                0.4,
                PaneTreeNode::leaf("top"),
                PaneTreeNode::leaf("bottom"),
            ),
        );
        let mut leaves = Vec::new();
        tree.visit_leaves(|pane_id| leaves.push(*pane_id));
        assert_eq!(leaves, ["left", "top", "bottom"]);
        let mut splits = Vec::new();
        tree.visit_splits(|split_id, axis, ratio| splits.push((*split_id, axis, ratio)));
        assert_eq!(
            splits,
            [
                ("root", SplitAxis::Horizontal, 0.6),
                ("right-stack", SplitAxis::Vertical, 0.4),
            ]
        );
    }

    #[test]
    fn pane_tree_clamps_ratio_at_the_public_boundary() {
        let PaneTreeNode::Split { ratio, .. } = PaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            2.0,
            PaneTreeNode::<&str, &str>::leaf("left"),
            PaneTreeNode::leaf("right"),
        ) else {
            panic!("split")
        };
        assert_eq!(ratio, 0.95);
        assert_eq!(split_ratio_portions(ratio), (950, 50));
    }
}
