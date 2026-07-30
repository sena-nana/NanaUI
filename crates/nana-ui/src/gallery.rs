use iced::widget::{
    button, checkbox, column, container, mouse_area, progress_bar, row, rule, scrollable, slider,
    space, stack, text, text_editor, text_input, toggler, tooltip,
};
use iced::{Alignment, Element, Length, Subscription, Task};

use crate::dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
use crate::icons::{Icon, icon, spinner_icon, status_indicator};
use crate::menu::{MenuConfirmation, MenuSelection};
use crate::overlay::ExclusiveOverlay;
use crate::selection::{SelectionMove, SingleSelection};
use crate::shell::AppTitleBar;
use crate::theme::{Colors, ThemeMode, UI_METRICS, ui_font};
use crate::tooltip::TooltipConfig;
use crate::widgets::{
    ButtonKind, CardKind, SEGMENTED_CONTROL_INSET, button_style, canvas_style, card_style,
    checkbox_style, dialog_close_style, dialog_scrim_style, dialog_surface_style,
    interactive_card_style, list_item_style, menu_item_style, menu_surface_style, panel_style,
    progress_style, scrollable_style, segmented_button_style, segmented_surface_style,
    selection_button_style, slider_style, text_editor_style, text_input_style, toggler_style,
    tooltip_style, vertical_scrollbar,
};
use crate::window_chrome::{WindowChromeEvent, WindowChromeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryTab {
    Controls,
    Surfaces,
    Feedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceView {
    Overview,
    Nodes,
}

impl SurfaceView {
    const fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Nodes => 1,
        }
    }

    const fn from_index(index: usize) -> Self {
        if index == 1 {
            Self::Nodes
        } else {
            Self::Overview
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GalleryMessage {
    ToggleTheme,
    SelectTab(GalleryTab),
    PrimaryAction,
    ToggleLoading,
    LoadingTick,
    InputChanged(String),
    ToggleCheck(bool),
    ToggleSwitch(bool),
    SetSlider(u8),
    SelectListItem(usize),
    SelectSurfaceCard(usize),
    SelectSurfaceView(SurfaceView),
    NavigateSurfaceView(SelectionMove),
    ToggleDialog,
    ConfirmDialog,
    RequestDialogClose(DialogCloseTrigger),
    DismissOverlay,
    OverlayInteraction,
    EditText(text_editor::Action),
    ToggleContextMenu,
    ContextAction(ContextAction),
    WindowChrome(WindowChromeEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Duplicate,
    Rename,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryOverlay {
    ContextMenu,
    Dialog,
}

fn overlay_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(GalleryMessage::RequestDialogClose(
            DialogCloseTrigger::Escape,
        )),
        _ => None,
    }
}

fn surface_selection_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    let movement = match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(named),
            ..
        }) => match named {
            iced::keyboard::key::Named::ArrowLeft => SelectionMove::Previous,
            iced::keyboard::key::Named::ArrowRight => SelectionMove::Next,
            iced::keyboard::key::Named::Home => SelectionMove::First,
            iced::keyboard::key::Named::End => SelectionMove::Last,
            _ => return None,
        },
        _ => return None,
    };
    Some(GalleryMessage::NavigateSurfaceView(movement))
}

#[derive(Debug)]
pub struct GalleryState {
    theme: ThemeMode,
    tab: GalleryTab,
    input: String,
    checked: bool,
    switched: bool,
    loading: bool,
    loading_ticks: u8,
    slider: u8,
    selected_item: usize,
    selected_surface_card: usize,
    surface_selection: SingleSelection,
    overlay: ExclusiveOverlay<GalleryOverlay>,
    dialog_policy: DialogClosePolicy,
    menu_confirmation: MenuConfirmation<ContextAction>,
    context_action: Option<ContextAction>,
    preview_refreshes: u32,
    editor: text_editor::Content,
    primary_clicks: u32,
    window_chrome: WindowChromeState,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryState {
    pub fn new() -> Self {
        Self {
            theme: ThemeMode::Dark,
            tab: GalleryTab::Controls,
            input: String::new(),
            checked: true,
            switched: true,
            loading: false,
            loading_ticks: 0,
            slider: 58,
            selected_item: 0,
            selected_surface_card: 0,
            surface_selection: SingleSelection::new(0),
            overlay: ExclusiveOverlay::new(),
            dialog_policy: DialogClosePolicy::default(),
            menu_confirmation: MenuConfirmation::new(),
            context_action: None,
            preview_refreshes: 0,
            editor: text_editor::Content::with_text("节点说明\n保持预览连续更新"),
            primary_clicks: 0,
            window_chrome: WindowChromeState::default(),
        }
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn editor_enabled(&self) -> bool {
        self.checked && self.switched
    }

    pub fn subscription(&self) -> Subscription<GalleryMessage> {
        let interaction = if self.overlay.is_open() {
            iced::event::listen_with(overlay_event)
        } else if self.tab == GalleryTab::Surfaces {
            iced::event::listen_with(surface_selection_event)
        } else {
            Subscription::none()
        };
        let loading = if self.loading {
            iced::time::every(iced::time::Duration::from_millis(100))
                .map(|_| GalleryMessage::LoadingTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            interaction,
            loading,
            WindowChromeState::subscription().map(GalleryMessage::WindowChrome),
        ])
    }

    pub fn update_windowed(&mut self, message: GalleryMessage) -> Task<GalleryMessage> {
        if let GalleryMessage::WindowChrome(event) = message {
            return self
                .window_chrome
                .update_iced(event)
                .map(GalleryMessage::WindowChrome);
        }
        self.update(message);
        Task::none()
    }

    pub fn update(&mut self, message: GalleryMessage) {
        match message {
            GalleryMessage::WindowChrome(event) => {
                self.window_chrome.update(event);
            }
            GalleryMessage::ToggleTheme => self.theme = self.theme.toggle(),
            GalleryMessage::SelectTab(tab) => {
                self.tab = tab;
                self.overlay.dismiss();
                self.menu_confirmation.clear();
            }
            GalleryMessage::PrimaryAction => {
                self.primary_clicks = self.primary_clicks.saturating_add(1);
            }
            GalleryMessage::ToggleLoading => {
                self.loading = true;
                self.loading_ticks = 0;
            }
            GalleryMessage::LoadingTick => {
                if self.loading {
                    self.loading_ticks = self.loading_ticks.saturating_add(1);
                    if self.loading_ticks >= 12 {
                        self.loading = false;
                        self.loading_ticks = 0;
                    }
                }
            }
            GalleryMessage::InputChanged(input) => self.input = input,
            GalleryMessage::ToggleCheck(value) => self.checked = value,
            GalleryMessage::ToggleSwitch(value) => self.switched = value,
            GalleryMessage::SetSlider(value) => self.slider = value.min(100),
            GalleryMessage::SelectListItem(index) => self.selected_item = index,
            GalleryMessage::SelectSurfaceCard(index) => self.selected_surface_card = index,
            GalleryMessage::SelectSurfaceView(view) => {
                self.surface_selection.select(view.index(), &[true, true]);
            }
            GalleryMessage::NavigateSurfaceView(movement) => {
                self.surface_selection.navigate(movement, &[true, true]);
            }
            GalleryMessage::ToggleDialog => {
                self.menu_confirmation.clear();
                self.overlay.toggle(GalleryOverlay::Dialog);
            }
            GalleryMessage::ConfirmDialog => {
                if self.overlay.contains(&GalleryOverlay::Dialog) {
                    self.preview_refreshes = self.preview_refreshes.saturating_add(1);
                    self.context_action = None;
                    self.overlay.dismiss();
                }
            }
            GalleryMessage::RequestDialogClose(trigger) => {
                if self.overlay.contains(&GalleryOverlay::Dialog) {
                    if self.dialog_policy.allows(trigger) {
                        self.overlay.dismiss();
                    }
                } else if trigger == DialogCloseTrigger::Escape {
                    self.overlay.dismiss();
                    self.menu_confirmation.clear();
                }
            }
            GalleryMessage::DismissOverlay => {
                self.overlay.dismiss();
                self.menu_confirmation.clear();
            }
            GalleryMessage::OverlayInteraction => {}
            GalleryMessage::EditText(action) => self.editor.perform(action),
            GalleryMessage::ToggleContextMenu => {
                self.menu_confirmation.clear();
                self.overlay.toggle(GalleryOverlay::ContextMenu);
            }
            GalleryMessage::ContextAction(action) => {
                if !self.overlay.contains(&GalleryOverlay::ContextMenu) {
                    return;
                }
                let requires_confirmation = action == ContextAction::Remove;
                if let MenuSelection::Confirmed(action) =
                    self.menu_confirmation.select(action, requires_confirmation)
                {
                    self.apply_context_action(action);
                }
            }
        }
    }

    fn apply_context_action(&mut self, action: ContextAction) {
        self.context_action = Some(action);
        self.overlay.dismiss();
        self.menu_confirmation.clear();
        match action {
            ContextAction::Duplicate => {
                self.primary_clicks = self.primary_clicks.saturating_add(1);
            }
            ContextAction::Rename => {
                self.editor = text_editor::Content::with_text("已重命名节点");
            }
            ContextAction::Remove => {
                self.selected_item = 0;
            }
        }
    }

    pub fn view(&self) -> Element<'_, GalleryMessage> {
        let colors = self.theme.colors();
        let theme_icon = match self.theme {
            ThemeMode::Dark => Icon::Appearance,
            ThemeMode::Light => Icon::Moon,
        };
        let header_actions = row![
            text("组件").size(11).color(colors.muted),
            button(icon(theme_icon, 14.0, colors.accent))
                .on_press(GalleryMessage::ToggleTheme)
                .width(Length::Fixed(UI_METRICS.icon_button_size))
                .height(Length::Fixed(UI_METRICS.icon_button_size))
                .padding(0)
                .style(button_style(colors, ButtonKind::Text)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let header = AppTitleBar::new("Component Gallery", colors)
            .trailing(header_actions)
            .window_chrome(&self.window_chrome, GalleryMessage::WindowChrome)
            .view();

        let tabs = row![
            self.tab_button("控件", GalleryTab::Controls, colors),
            self.tab_button("表面", GalleryTab::Surfaces, colors),
            self.tab_button("反馈", GalleryTab::Feedback, colors),
        ]
        .spacing(2)
        .height(Length::Fixed(UI_METRICS.selection_height));
        let tabs = column![
            container(tabs)
                .height(Length::Fixed(UI_METRICS.selection_height))
                .padding([0.0, UI_METRICS.panel_padding_x]),
            rule::horizontal(1).style(move |_theme| iced::widget::rule::Style {
                color: colors.border,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
                snap: true,
            }),
        ];

        let content = match self.tab {
            GalleryTab::Controls => self.controls(colors),
            GalleryTab::Surfaces => self.surfaces(colors),
            GalleryTab::Feedback => self.feedback(colors),
        };

        let base = container(column![header, tabs, content])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(colors.background)
                    .color(colors.text)
            });

        if self.overlay.contains(&GalleryOverlay::Dialog) {
            stack![base, self.dialog(colors)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        }
    }

    fn tab_button<'a>(
        &'a self,
        label: &'a str,
        tab: GalleryTab,
        colors: Colors,
    ) -> iced::widget::Button<'a, GalleryMessage> {
        button(text(label).size(13))
            .height(Length::Fixed(UI_METRICS.selection_height))
            .padding([0.0, UI_METRICS.selection_padding_x])
            .on_press(GalleryMessage::SelectTab(tab))
            .style(selection_button_style(colors, self.tab == tab))
    }

    fn controls(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let input_invalid = self.input.trim().is_empty();
        let editor_invalid = self.editor.text().trim().chars().count() < 4;
        let editor_enabled = self.editor_enabled();
        let loading_content = if self.loading {
            row![
                spinner_icon(self.loading_ticks, 14.0, colors.accent),
                text("处理中").size(13),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
        } else {
            row![text("加载").size(13)]
        };
        let loading_button = button(loading_content)
            .height(Length::Fixed(UI_METRICS.control_height))
            .padding([0.0, UI_METRICS.control_padding_x])
            .style(button_style(colors, ButtonKind::Text));
        let loading_button = if self.loading {
            loading_button
        } else {
            loading_button.on_press(GalleryMessage::ToggleLoading)
        };
        let buttons = container(
            column![
                text("操作").size(12).color(colors.muted),
                row![
                    button(text("次要").size(13))
                        .height(Length::Fixed(UI_METRICS.control_height))
                        .padding([0.0, UI_METRICS.control_padding_x])
                        .on_press(GalleryMessage::PrimaryAction)
                        .style(button_style(colors, ButtonKind::Subtle)),
                    button(text("主要").size(13))
                        .height(Length::Fixed(UI_METRICS.control_height))
                        .padding([0.0, UI_METRICS.control_padding_x])
                        .on_press(GalleryMessage::PrimaryAction)
                        .style(button_style(colors, ButtonKind::Primary)),
                    button(text("禁用").size(13))
                        .height(Length::Fixed(UI_METRICS.control_height))
                        .padding([0.0, UI_METRICS.control_padding_x])
                        .style(button_style(colors, ButtonKind::Subtle)),
                    loading_button,
                    button(icon(Icon::Add, 14.0, colors.text))
                        .width(Length::Fixed(UI_METRICS.icon_button_size))
                        .height(Length::Fixed(UI_METRICS.icon_button_size))
                        .padding(0)
                        .on_press(GalleryMessage::PrimaryAction)
                        .style(button_style(colors, ButtonKind::Ghost)),
                ]
                .spacing(8),
                text(format!("主要操作已触发 {} 次", self.primary_clicks))
                    .size(10)
                    .color(colors.faint),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let fields = container(
            column![
                text("节点名称 *")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                text_input("输入节点名称", &self.input)
                    .on_input(GalleryMessage::InputChanged)
                    .padding([UI_METRICS.field_padding_y, UI_METRICS.field_padding_x,])
                    .size(13)
                    .width(Length::Fill)
                    .style(text_input_style(colors, input_invalid)),
                text(if input_invalid {
                    "请输入名称"
                } else {
                    "名称可用"
                })
                .size(12)
                .color(if input_invalid {
                    colors.danger
                } else {
                    colors.success
                }),
                text_input("", "系统节点")
                    .padding([UI_METRICS.field_padding_y, UI_METRICS.field_padding_x,])
                    .size(13)
                    .width(Length::Fill)
                    .style(text_input_style(colors, false)),
            ]
            .spacing(5),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let editor_toggle = toggler(self.switched)
            .label("允许编辑说明")
            .size(16)
            .spacing(8)
            .text_size(13)
            .style(toggler_style(colors, false));
        let editor_toggle = if self.checked {
            editor_toggle.on_toggle(GalleryMessage::ToggleSwitch)
        } else {
            editor_toggle
        };
        let toggles = container(
            column![
                text("节点设置").size(12).color(colors.muted),
                checkbox(self.checked)
                    .label("启用当前节点")
                    .on_toggle(GalleryMessage::ToggleCheck)
                    .size(16)
                    .spacing(8)
                    .text_size(13)
                    .style(checkbox_style(colors, false)),
                editor_toggle,
                row![
                    text("强度").size(11),
                    slider(0..=100, self.slider, GalleryMessage::SetSlider)
                        .height(16)
                        .style(slider_style(colors)),
                    text(format!("{}%", self.slider))
                        .size(10)
                        .color(colors.accent),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let editor = text_editor(&self.editor)
            .placeholder("输入说明")
            .height(Length::Fixed(96.0))
            .padding(9)
            .size(13)
            .line_height(iced::widget::text::LineHeight::Relative(1.45))
            .style(text_editor_style(colors, editor_invalid));
        let editor = if editor_enabled {
            editor.on_action(GalleryMessage::EditText)
        } else {
            editor
        };
        let text_area = container(
            column![
                text("节点说明")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                editor,
                text(if editor_invalid {
                    "请至少输入 4 个字符"
                } else if editor_enabled {
                    "说明可编辑"
                } else if !self.checked {
                    "节点停用时说明不可编辑"
                } else {
                    "说明已锁定"
                })
                .size(12)
                .color(if editor_invalid {
                    colors.danger
                } else {
                    colors.muted
                }),
            ]
            .spacing(5),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let items = [
            ("输入节点", false),
            ("颜色处理", false),
            ("预览输出", false),
            ("位置变换", false),
            ("尺寸约束", false),
            ("透明度", false),
            ("混合模式", false),
            ("实验节点", true),
            ("描边设置", false),
            ("阴影设置", false),
            ("遮罩输入", false),
            ("纹理采样", false),
            ("参数映射", false),
            ("时间轴", false),
            ("输出检查", false),
            ("发布设置", false),
        ];
        let mut list = column![].spacing(4);
        for (index, (label, disabled)) in items.into_iter().enumerate() {
            let selected = self.selected_item == index;
            let item = button(
                row![
                    status_indicator(
                        selected,
                        10.0,
                        if selected {
                            colors.accent
                        } else {
                            colors.faint
                        },
                    ),
                    text(label).size(13),
                    space().width(Length::Fill),
                    text(if disabled { "不可用" } else { "" })
                        .size(11)
                        .color(colors.muted),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fixed(UI_METRICS.selection_height))
            .padding([
                UI_METRICS.list_item_padding_y,
                UI_METRICS.list_item_padding_x,
            ])
            .style(list_item_style(colors, selected));
            list = list.push(if disabled {
                item
            } else {
                item.on_press(GalleryMessage::SelectListItem(index))
            });
        }
        let list = container(
            column![
                text("节点列表").size(12).color(colors.muted),
                scrollable(list)
                    .direction(vertical_scrollbar())
                    .style(scrollable_style(colors))
                    .height(Length::Fill),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .height(Length::Fill)
        .style(panel_style(colors));

        container(
            column![
                row![buttons, fields, toggles].spacing(10),
                row![text_area, list].spacing(10)
            ]
            .spacing(10),
        )
        .padding(iced::Padding {
            top: 0.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors))
        .into()
    }

    fn surfaces(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let selected_view = SurfaceView::from_index(self.surface_selection.selected());
        let card = |title: &'static str, detail: &'static str, kind| {
            container(
                column![
                    text(title).size(13).color(colors.text),
                    text(detail).size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::FillPortion(1))
            .height(Length::Fixed(96.0))
            .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
            .style(card_style(colors, kind))
        };
        let preview = match selected_view {
            SurfaceView::Overview => row![
                card("基础表面", "主工作区内容层", CardKind::Surface),
                card("抬升表面", "侧栏与工具面板", CardKind::Raised),
                card("选中表面", "当前激活的内容", CardKind::Selected),
            ],
            SurfaceView::Nodes => {
                let nodes = [
                    ("输入节点", "接收模型参数", false),
                    ("颜色处理", "调整颜色与透明度", false),
                    ("预览输出", "等待渲染器接入", true),
                ];
                let mut cards = row![].spacing(10);
                for (index, (title, detail, disabled)) in nodes.into_iter().enumerate() {
                    let selected = self.selected_surface_card == index;
                    let node = button(
                        column![
                            text(title).size(13),
                            text(detail).size(11).color(colors.muted),
                        ]
                        .spacing(6),
                    )
                    .width(Length::FillPortion(1))
                    .height(Length::Fixed(96.0))
                    .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                    .align_x(iced::alignment::Horizontal::Left)
                    .style(interactive_card_style(colors, selected));
                    cards = cards.push(if disabled {
                        node
                    } else {
                        node.on_press(GalleryMessage::SelectSurfaceCard(index))
                    });
                }
                cards
            }
        }
        .spacing(10);
        let segmented = container(
            row![
                button(text("概览").size(13))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.selection_padding_x])
                    .on_press(GalleryMessage::SelectSurfaceView(SurfaceView::Overview))
                    .style(segmented_button_style(
                        colors,
                        selected_view == SurfaceView::Overview
                    )),
                button(text("节点").size(13))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.selection_padding_x])
                    .on_press(GalleryMessage::SelectSurfaceView(SurfaceView::Nodes))
                    .style(segmented_button_style(
                        colors,
                        selected_view == SurfaceView::Nodes
                    )),
            ]
            .spacing(2),
        )
        .height(Length::Fixed(UI_METRICS.selection_height))
        .padding(SEGMENTED_CONTROL_INSET)
        .style(segmented_surface_style(colors));

        container(
            column![
                text("表面层级").size(14).color(colors.text),
                text("基础、抬升与选中状态").size(11).color(colors.muted),
                container(
                    row![
                        text("表面预览").size(12),
                        space().width(Length::Fill),
                        segmented,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                .style(panel_style(colors)),
                preview,
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 0.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors))
        .into()
    }

    fn feedback(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let progress = if self.loading { 72.0 } else { 0.0 };
        let tooltip_config = TooltipConfig::default();
        let action_status = match self.context_action {
            Some(ContextAction::Duplicate) => "已复制".to_owned(),
            Some(ContextAction::Rename) => "已重命名".to_owned(),
            Some(ContextAction::Remove) => "已移除".to_owned(),
            None if self.preview_refreshes > 0 => {
                format!("预览已刷新 {} 次", self.preview_refreshes)
            }
            None => "等待操作".to_owned(),
        };
        let actions = container(
            column![
                button(
                    text(if self.overlay.contains(&GalleryOverlay::Dialog) {
                        "关闭对话框"
                    } else {
                        "打开对话框"
                    })
                    .size(11),
                )
                .width(Length::Fill)
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(GalleryMessage::ToggleDialog)
                .style(button_style(colors, ButtonKind::Primary)),
                button(text("更多操作").size(11))
                    .width(Length::Fill)
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ToggleContextMenu)
                    .style(button_style(colors, ButtonKind::Subtle)),
                tooltip(
                    container(icon(Icon::About, 13.0, colors.muted))
                        .width(Length::Fixed(UI_METRICS.icon_button_size))
                        .height(Length::Fixed(UI_METRICS.icon_button_size))
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center),
                    container(
                        text(format!("当前状态：{action_status}"))
                            .size(11)
                            .line_height(iced::widget::text::LineHeight::Absolute(iced::Pixels(
                                16.0
                            ),)),
                    )
                    .width(tooltip_config.max_width)
                    .padding([4, 7]),
                    tooltip_config.placement.into(),
                )
                .gap(tooltip_config.gap)
                .padding(tooltip_config.viewport_padding)
                .delay(iced::time::Duration::from_millis(tooltip_config.delay_ms,))
                .snap_within_viewport(true)
                .style(tooltip_style(colors)),
            ]
            .spacing(8)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .width(Length::Fixed(140.0))
        .height(Length::Fixed(124.0))
        .padding([8, 10])
        .style(panel_style(colors));

        let content = container(
            column![
                text("Feedback").size(14).color(colors.text),
                row![
                    container(
                        column![
                            text(if self.loading {
                                "处理中"
                            } else {
                                "已完成"
                            })
                            .size(13),
                            progress_bar(0.0..=100.0, progress)
                                .girth(6)
                                .style(progress_style(colors)),
                        ]
                        .spacing(8),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(124.0))
                    .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                    .style(panel_style(colors)),
                    actions,
                ]
                .spacing(10)
                .align_y(Alignment::Start),
                text(action_status).size(10).color(colors.muted),
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 0.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors));

        if self.overlay.contains(&GalleryOverlay::ContextMenu) {
            stack![content, self.context_menu(colors)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            content.into()
        }
    }

    fn context_menu(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let remove_pending = self.menu_confirmation.pending() == Some(&ContextAction::Remove);
        let menu = mouse_area(
            container(
                column![
                    button(text("复制节点").size(13))
                        .width(Length::Fill)
                        .height(Length::Fixed(UI_METRICS.compact_control_height))
                        .padding([0.0, UI_METRICS.control_padding_x])
                        .align_x(iced::alignment::Horizontal::Left)
                        .on_press(GalleryMessage::ContextAction(ContextAction::Duplicate))
                        .style(menu_item_style(colors, false, false)),
                    button(text("重命名节点").size(13))
                        .width(Length::Fill)
                        .height(Length::Fixed(UI_METRICS.compact_control_height))
                        .padding([0.0, UI_METRICS.control_padding_x])
                        .align_x(iced::alignment::Horizontal::Left)
                        .on_press(GalleryMessage::ContextAction(ContextAction::Rename))
                        .style(menu_item_style(colors, false, false)),
                    button(
                        text(if remove_pending {
                            "再次点击确认移除"
                        } else {
                            "移除节点"
                        })
                        .size(13)
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .align_x(iced::alignment::Horizontal::Left)
                    .on_press(GalleryMessage::ContextAction(ContextAction::Remove))
                    .style(menu_item_style(colors, true, remove_pending)),
                ]
                .spacing(1),
            )
            .width(Length::Fixed(180.0))
            .padding(4)
            .style(menu_surface_style(colors)),
        )
        .on_press(GalleryMessage::OverlayInteraction);

        mouse_area(
            container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_right(Length::Fill)
                .align_top(Length::Fill)
                .padding(iced::Padding {
                    top: 112.0,
                    right: 24.0,
                    bottom: 0.0,
                    left: 0.0,
                }),
        )
        .on_press(GalleryMessage::DismissOverlay)
        .into()
    }

    fn dialog(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let header = container(
            row![
                column![
                    text("刷新预览").size(14).color(colors.text),
                    text("确认后重新执行当前预览").size(12).color(colors.muted),
                ]
                .spacing(4)
                .width(Length::Fill),
                button(icon(Icon::Close, 14.0, colors.muted))
                    .width(Length::Fixed(UI_METRICS.icon_button_size))
                    .height(Length::Fixed(UI_METRICS.icon_button_size))
                    .padding(0)
                    .on_press(GalleryMessage::RequestDialogClose(
                        DialogCloseTrigger::CloseButton
                    ))
                    .style(dialog_close_style(colors)),
            ]
            .spacing(12)
            .align_y(Alignment::Start),
        )
        .padding(iced::Padding {
            top: 14.0,
            right: 16.0,
            bottom: 8.0,
            left: 16.0,
        });

        let body = container(
            text("将使用当前节点和参数重新生成预览。")
                .size(13)
                .color(colors.text),
        )
        .width(Length::Fill)
        .padding(iced::Padding {
            top: 8.0,
            right: 16.0,
            bottom: 14.0,
            left: 16.0,
        });

        let footer = container(
            row![
                space().width(Length::Fill),
                button(text("取消").size(13))
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::RequestDialogClose(
                        DialogCloseTrigger::CloseButton
                    ))
                    .style(button_style(colors, ButtonKind::Ghost)),
                button(text("确认刷新").size(13))
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ConfirmDialog)
                    .style(button_style(colors, ButtonKind::Primary)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(iced::Padding {
            top: 0.0,
            right: 16.0,
            bottom: 14.0,
            left: 16.0,
        });

        let dialog = mouse_area(
            container(column![header, body, footer])
                .width(Length::Fixed(DialogSize::Default.max_width()))
                .style(dialog_surface_style(colors)),
        )
        .on_press(GalleryMessage::OverlayInteraction);

        mouse_area(
            container(dialog)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .align_top(Length::Fill)
                .padding(iced::Padding {
                    top: 90.0,
                    right: 16.0,
                    bottom: 16.0,
                    left: 16.0,
                })
                .style(dialog_scrim_style(colors)),
        )
        .on_press(GalleryMessage::RequestDialogClose(
            DialogCloseTrigger::Outside,
        ))
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContextAction, GalleryMessage, GalleryOverlay, GalleryState, GalleryTab, SurfaceView,
    };
    use crate::selection::SelectionMove;

    #[test]
    fn gallery_interactions_update_real_state() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::PrimaryAction);
        state.update(GalleryMessage::ToggleLoading);
        state.update(GalleryMessage::InputChanged("Node".to_owned()));
        state.update(GalleryMessage::SelectTab(GalleryTab::Feedback));
        state.update(GalleryMessage::ToggleContextMenu);
        state.update(GalleryMessage::ContextAction(ContextAction::Rename));

        assert_eq!(state.primary_clicks, 1);
        assert!(state.loading);
        assert_eq!(state.input, "Node");
        assert_eq!(state.tab, GalleryTab::Feedback);
        assert!(!state.overlay.is_open());
        assert_eq!(state.editor.text(), "已重命名节点");
    }

    #[test]
    fn gallery_overlays_are_mutually_exclusive_and_dismissible() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleDialog);
        assert!(state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::ToggleContextMenu);
        assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
        assert!(!state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::DismissOverlay);
        assert!(!state.overlay.is_open());
    }

    #[test]
    fn destructive_menu_action_requires_confirmation() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::SelectListItem(2));
        state.update(GalleryMessage::SelectTab(GalleryTab::Feedback));
        state.update(GalleryMessage::ToggleContextMenu);

        state.update(GalleryMessage::ContextAction(ContextAction::Remove));
        assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
        assert_eq!(
            state.menu_confirmation.pending(),
            Some(&ContextAction::Remove)
        );
        assert_eq!(state.selected_item, 2);

        state.update(GalleryMessage::ContextAction(ContextAction::Remove));
        assert!(!state.overlay.is_open());
        assert_eq!(state.context_action, Some(ContextAction::Remove));
        assert_eq!(state.selected_item, 0);
    }

    #[test]
    fn dialog_confirmation_executes_and_closes_the_overlay() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleDialog);
        assert!(state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::ConfirmDialog);
        assert!(!state.overlay.is_open());
        assert_eq!(state.preview_refreshes, 1);
    }

    #[test]
    fn segmented_surface_view_supports_click_and_roving_selection() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::SelectSurfaceView(SurfaceView::Nodes));
        assert_eq!(state.surface_selection.selected(), 1);

        state.update(GalleryMessage::SelectSurfaceCard(1));
        assert_eq!(state.selected_surface_card, 1);

        state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Next));
        assert_eq!(state.surface_selection.selected(), 0);
        state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Last));
        assert_eq!(state.surface_selection.selected(), 1);
    }

    #[test]
    fn loading_state_blocks_until_the_async_cycle_finishes() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleLoading);
        assert!(state.loading);

        for _ in 0..11 {
            state.update(GalleryMessage::LoadingTick);
            assert!(state.loading);
        }
        state.update(GalleryMessage::LoadingTick);
        assert!(!state.loading);
        assert_eq!(state.loading_ticks, 0);
    }

    #[test]
    fn node_and_edit_switches_control_editor_availability() {
        let mut state = GalleryState::new();
        assert!(state.editor_enabled());

        state.update(GalleryMessage::ToggleCheck(false));
        assert!(!state.editor_enabled());

        state.update(GalleryMessage::ToggleCheck(true));
        state.update(GalleryMessage::ToggleSwitch(false));
        assert!(!state.editor_enabled());
    }
}
