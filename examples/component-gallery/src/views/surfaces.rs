use super::*;
use iced::widget::column;

impl GalleryState {
    #[allow(dead_code)]
    pub(super) fn surfaces(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let selected_view = SurfaceView::from_index(self.surface_selection.selected());
        let card = |title: &'static str, detail: &'static str, kind| {
            container(
                UiCard::new(
                    column![
                        text(title).size(13).color(colors.text),
                        text(detail).size(11).color(colors.muted),
                    ]
                    .spacing(6),
                )
                .kind(kind)
                .view(colors),
            )
            .width(Length::FillPortion(1))
            .height(Length::Fixed(96.0))
        };
        let surface_content = match selected_view {
            SurfaceView::Overview => row![
                card("基础表面", "主工作区内容层", CardKind::Surface),
                card("抬升表面", "侧栏与工具面板", CardKind::Raised),
                card("选中表面", "当前激活的内容", CardKind::Selected),
            ],
            SurfaceView::Cards => {
                let cards_data = [
                    ("默认卡片", "普通内容容器", false),
                    ("交互卡片", "支持选择操作", false),
                    ("禁用卡片", "不可进行操作", true),
                ];
                let mut cards = row![].spacing(10);
                for (index, (title, detail, disabled)) in cards_data.into_iter().enumerate() {
                    let selected = self.selected_surface_card == index;
                    let node = UiInteractiveCard::new(
                        column![
                            text(title).size(13),
                            text(detail).size(11).color(colors.muted),
                        ]
                        .spacing(6),
                    )
                    .selected(selected)
                    .disabled(disabled)
                    .on_select(GalleryMessage::SelectSurfaceCard(index))
                    .view(colors);
                    cards = cards.push(
                        container(node)
                            .width(Length::FillPortion(1))
                            .height(Length::Fixed(96.0)),
                    );
                }
                cards
            }
        }
        .spacing(10);
        let segmented = UiTabs::new(
            selected_view,
            [
                SelectionOption::new(SurfaceView::Overview, "概览"),
                SelectionOption::new(SurfaceView::Cards, "卡片"),
            ],
            GalleryMessage::SelectSurfaceView,
        )
        .view(colors);
        let tree = UiTreeView::new(
            [
                TreeNode::branch(
                    "src".to_owned(),
                    "src",
                    self.tree_expanded,
                    [
                        TreeNode::leaf("src/lib.rs".to_owned(), "lib.rs")
                            .icon(Icon::File)
                            .selected(self.tree_selected == "src/lib.rs"),
                        TreeNode::leaf("src/main.rs".to_owned(), "main.rs")
                            .icon(Icon::File)
                            .selected(self.tree_selected == "src/main.rs"),
                    ],
                )
                .icon(Icon::Folder)
                .selected(self.tree_selected == "src"),
                TreeNode::leaf("README.md".to_owned(), "README.md")
                    .icon(Icon::File)
                    .selected(self.tree_selected == "README.md"),
            ],
            GalleryMessage::TreeView,
            self.theme_tokens(),
        )
        .view();
        let pane_tabs: Element<'_, GalleryMessage> = text(if self.pane_chrome_item_open {
            "main.rs"
        } else {
            "空窗格"
        })
        .size(11)
        .color(colors.text)
        .into();
        let pane_tree = if !self.pane_chrome_item_open {
            PaneTreeNode::leaf("empty")
        } else if self.pane_chrome_split {
            PaneTreeNode::split(
                "editor-split",
                SplitAxis::Horizontal,
                0.5,
                PaneTreeNode::leaf("left"),
                PaneTreeNode::leaf("right"),
            )
        } else {
            PaneTreeNode::leaf("editor")
        };
        let pane_body = PaneTree::new(
            pane_tree,
            move |pane_id| {
                let (label, color) = match *pane_id {
                    "empty" => ("Item 已关闭", colors.muted),
                    "left" => ("左侧编辑器", colors.text),
                    "right" => ("右侧编辑器", colors.text),
                    _ => ("编辑器内容", colors.text),
                };
                container(text(label).size(11).color(color))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill)
                    .into()
            },
            |_split_id, axis, ratio, first, second| {
                ratio_pane_split(axis, ratio, first, second, self.theme_tokens())
            },
        )
        .view();
        let pane_body: Element<'_, GalleryMessage> = container(pane_body)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        let mut pane_actions = Vec::new();
        if self.pane_chrome_item_open && !self.pane_chrome_split {
            pane_actions.push(PaneChromeAction::new(
                PaneChromeActionKind::SplitHorizontal,
                "左右分栏",
                GalleryMessage::PaneChrome(PaneChromeActionKind::SplitHorizontal),
            ));
        }
        if self.pane_chrome_item_open {
            pane_actions.push(
                PaneChromeAction::new(
                    PaneChromeActionKind::CloseItem,
                    "关闭 Item",
                    GalleryMessage::PaneChrome(PaneChromeActionKind::CloseItem),
                )
                .icon(Icon::Close),
            );
        }
        let pane = PaneChrome::new(pane_tabs, pane_body, pane_actions, self.theme_tokens()).view();

        container(
            column![
                text("表面层级").size(14).color(colors.text),
                text("基础、抬升与选中状态").size(11).color(colors.muted),
                container(
                    row![
                        text("表面状态").size(12),
                        space().width(Length::Fill),
                        segmented,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                .style(panel_style(colors)),
                surface_content,
                text("层级树").size(14).color(colors.text),
                text("稳定节点 ID 驱动展开与选择")
                    .size(11)
                    .color(colors.muted),
                container(tree)
                    .width(Length::Fill)
                    .padding(8)
                    .style(panel_style(colors)),
                text("Pane 组合").size(14).color(colors.text),
                text("动作只在具备真实 handler 时出现")
                    .size(11)
                    .color(colors.muted),
                container(pane)
                    .width(Length::Fill)
                    .height(Length::Fixed(140.0))
                    .style(panel_style(colors)),
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors))
        .into()
    }
}
