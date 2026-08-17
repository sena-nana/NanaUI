use super::*;
use iced::widget::column;

impl GalleryState {
    #[allow(dead_code)]
    pub(super) fn controls(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let input_invalid = self.input.trim().is_empty();
        let editor_invalid = self.editor.text().trim().chars().count() < 4;
        let editor_enabled = self.editor_enabled();
        let segmented = |size| {
            UiSegmentedControl::new(
                self.checked,
                [
                    SelectionOption::new(false, "关"),
                    SelectionOption::new(true, "开"),
                ],
                GalleryMessage::ToggleCheck,
            )
            .size(size)
            .view(tokens)
        };
        let input = |placeholder, size| {
            UiInput::new(placeholder, &self.input)
                .size(size)
                .on_input(GalleryMessage::InputChanged)
                .invalid(input_invalid)
                .view(tokens)
        };
        let dropdown = |placeholder, size| {
            UiDropdown::multiple(
                self.dropdown_values.iter().copied(),
                [
                    DropdownOption::new(0, "关闭"),
                    DropdownOption::new(50, "平衡"),
                    DropdownOption::new(100, "最大"),
                ],
                GalleryMessage::SetDropdown,
            )
            .size(size)
            .placeholder(placeholder)
            .width(Length::Fill)
            .view(tokens)
        };
        let loading_button = UiButton::label(if self.loading { "处理中" } else { "加载" })
            .kind(ButtonKind::Text)
            .on_press(GalleryMessage::ToggleLoading)
            .loading(self.loading, self.loading_ticks)
            .view(tokens);
        let buttons = container(
            column![
                text("三档操作").size(12).color(colors.muted),
                row![
                    UiButton::label("小")
                        .size(ControlSize::Small)
                        .on_press(GalleryMessage::PrimaryAction)
                        .kind(ButtonKind::Subtle)
                        .view(tokens),
                    UiButton::label("中")
                        .size(ControlSize::Medium)
                        .on_press(GalleryMessage::PrimaryAction)
                        .kind(ButtonKind::Primary)
                        .view(tokens),
                    UiButton::label("大")
                        .size(ControlSize::Large)
                        .on_press(GalleryMessage::PrimaryAction)
                        .kind(ButtonKind::Subtle)
                        .view(tokens),
                    loading_button,
                    UiIconButton::new("添加", Icon::Add)
                        .size(ControlSize::Small)
                        .on_press(GalleryMessage::PrimaryAction)
                        .view(tokens),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                row![
                    segmented(ControlSize::Small),
                    segmented(ControlSize::Medium),
                    segmented(ControlSize::Large),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                text(format!("主要操作已触发 {} 次", self.primary_clicks))
                    .size(10)
                    .color(colors.faint),
            ]
            .spacing(6),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(170.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(tokens));

        let fields = container(
            column![
                text("字段名称 *")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                row![
                    input("小", ControlSize::Small),
                    input("中", ControlSize::Medium),
                    input("大", ControlSize::Large),
                ]
                .spacing(6),
                UiInput::new("配对密钥", &self.input)
                    .secure(true)
                    .on_input(GalleryMessage::InputChanged)
                    .view(tokens),
                row![
                    dropdown("小", ControlSize::Small),
                    dropdown("中", ControlSize::Medium),
                    dropdown("大", ControlSize::Large),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
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
            ]
            .spacing(5),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(208.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(tokens));

        let editor_toggle = UiSwitch::new(self.switched, "允许编辑说明")
            .on_toggle(GalleryMessage::ToggleSwitch)
            .disabled(!self.checked)
            .view(tokens);
        let toggles = container(
            column![
                text("选择控件").size(12).color(colors.muted),
                UiCheckbox::new(self.checked, "启用选项")
                    .on_toggle(GalleryMessage::ToggleCheck)
                    .view(tokens),
                editor_toggle,
                row![
                    container(
                        UiRangeField::new(0.0..=100.0, f32::from(self.slider), |value| {
                            GalleryMessage::SetSlider(value.round() as u8)
                        },)
                        .label("强度")
                        .unit("%")
                        .view(tokens),
                    )
                    .width(Length::Fill),
                    container(
                        UiSearchDropdown::new(
                            &self.search_dropdown,
                            self.search_selection.as_ref(),
                            GalleryMessage::SelectSearchResult,
                        )
                        .placeholder("搜索选项")
                        .on_input(GalleryMessage::SearchDropdownInput)
                        .view(tokens),
                    )
                    .width(Length::Fixed(116.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(170.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(tokens));

        let editor = UiTextarea::new(&self.editor)
            .placeholder("输入说明")
            .height(96.0)
            .invalid(editor_invalid)
            .disabled(!editor_enabled)
            .on_action(GalleryMessage::EditText)
            .view(colors);
        let text_area = container(
            column![
                text("多行文本")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                editor,
                text(if editor_invalid {
                    "请至少输入 4 个字符"
                } else if editor_enabled {
                    "说明可编辑"
                } else if !self.checked {
                    "选项停用时不可编辑"
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

        let xy_pad = container(
            column![
                text("二维参数").size(12).color(colors.muted),
                UiXYPad::new(self.xy_pad, GalleryMessage::SetXYPad, colors)
                    .step(0.01)
                    .view(),
                text(format!("X {:.2} · Y {:.2}", self.xy_pad.x, self.xy_pad.y))
                    .size(11)
                    .color(colors.muted),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let items = [
            ("小档列表项", false, ControlSize::Small),
            ("中档列表项", false, ControlSize::Medium),
            ("大档列表项", false, ControlSize::Large),
            ("带辅助信息", false, ControlSize::Small),
            ("紧凑列表项", false, ControlSize::Small),
            ("长文本列表项", false, ControlSize::Small),
            ("可操作列表项", false, ControlSize::Small),
            ("禁用列表项", true, ControlSize::Small),
            ("普通状态", false, ControlSize::Small),
            ("悬停状态", false, ControlSize::Small),
            ("按下状态", false, ControlSize::Small),
            ("成功状态", false, ControlSize::Small),
            ("警告状态", false, ControlSize::Small),
            ("错误状态", false, ControlSize::Small),
            ("加载状态", false, ControlSize::Small),
            ("空状态", false, ControlSize::Small),
        ];
        let mut list = column![].spacing(4);
        for (index, (label, disabled, size)) in items.into_iter().enumerate() {
            let selected = self.selected_item == index;
            let item = UiListItem::label(label)
                .leading(status_indicator(
                    selected,
                    10.0,
                    if selected {
                        colors.accent
                    } else {
                        colors.faint
                    },
                ))
                .trailing(
                    text(if disabled { "不可用" } else { "" })
                        .size(11)
                        .color(colors.muted),
                )
                .size(size)
                .selected(selected)
                .disabled(disabled)
                .on_select(GalleryMessage::SelectListItem(index))
                .view(tokens);
            list = list.push(item);
        }
        let list = container(
            column![
                text("列表").size(12).color(colors.muted),
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
                row![text_area, xy_pad, list].spacing(10)
            ]
            .spacing(10),
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
