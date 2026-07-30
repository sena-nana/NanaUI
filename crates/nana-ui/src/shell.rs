use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Border, Color, Element, Length, Shadow, Theme, font};

use crate::geometry::TITLE_BAR_HEIGHT;
use crate::icons::{Icon, icon};
use crate::theme::{Colors, ThemeMode, ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{ButtonKind, button_style};
use crate::workspace_demo::{LayoutPreset, Message, WorkspaceState};

pub(crate) fn title_bar<'a>(state: &WorkspaceState, tokens: ThemeTokens) -> Element<'a, Message> {
    let colors = tokens.colors;
    let sidebar_toggle = button(icon(
        Icon::Sidebar,
        16.0,
        if state.sidebar_toggle_message().is_some() {
            colors.muted
        } else {
            colors.faint
        },
    ))
    .width(Length::Fixed(UI_METRICS.icon_button_size))
    .height(Length::Fixed(UI_METRICS.icon_button_size))
    .padding(0)
    .on_press_maybe(state.sidebar_toggle_message())
    .style(button_style(
        tokens,
        if state.sidebar_collapsed() {
            ButtonKind::Selected
        } else {
            ButtonKind::Ghost
        },
    ));
    let mut workspace_switcher = row![].spacing(2).align_y(Alignment::Center);
    for (preset, label) in [
        (LayoutPreset::Code, "Code"),
        (LayoutPreset::Github, "Github"),
        (LayoutPreset::Live2D, "Live2D"),
    ] {
        workspace_switcher = workspace_switcher.push(
            button(text(label).size(14))
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(Message::SelectLayout(preset))
                .style(layout_switcher_style(
                    tokens,
                    state.layout_preset() == preset,
                )),
        );
    }
    let leading = container(
        row![sidebar_toggle, workspace_switcher]
            .spacing(4)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .align_left(Length::Fill);
    let title = container(tracked_label(
        state.layout_title(),
        13.0,
        font::Weight::Semibold,
        0.2,
        colors.text,
    ))
    .width(Length::Fixed(140.0))
    .align_x(iced::alignment::Horizontal::Center);

    container(
        row![leading, title, space().width(Length::Fill)]
            .align_y(Alignment::Center)
            .spacing(0),
    )
    .width(Length::Fill)
    .height(Length::Fixed(TITLE_BAR_HEIGHT))
    .padding([0, 6])
    .align_y(iced::alignment::Vertical::Center)
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(colors.surface)
            .color(colors.text)
    })
    .into()
}

fn layout_switcher_style(
    tokens: ThemeTokens,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let colors = tokens.colors;
    move |_theme, status| {
        let background = match status {
            button::Status::Pressed => colors.active,
            button::Status::Hovered => colors.hover,
            button::Status::Active if selected => colors.hover,
            button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = if selected { colors.text } else { colors.muted };
        style.border = Border::default().rounded(tokens.metrics.radius_sm);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn app_title_bar<'a, Message>(
    title: &'a str,
    context: &'a str,
    theme: ThemeMode,
    toggle_theme: Message,
    leading_action: Option<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let theme_icon = match theme {
        ThemeMode::Dark => Icon::Appearance,
        ThemeMode::Light => Icon::Moon,
    };
    let leading = container(leading_action.unwrap_or_else(|| {
        space()
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .into()
    }))
    .width(Length::Fill)
    .align_left(Length::Fill);

    let title = container(tracked_label(
        title,
        13.0,
        font::Weight::Semibold,
        0.2,
        colors.text,
    ))
    .width(Length::Fixed(140.0))
    .align_x(iced::alignment::Horizontal::Center);

    let controls = row![
        text(context).size(11).color(colors.muted),
        button(icon(theme_icon, 14.0, colors.accent))
            .on_press(toggle_theme)
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .height(Length::Fixed(UI_METRICS.icon_button_size))
            .padding(0)
            .style(button_style(colors, ButtonKind::Text)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    container(
        row![
            leading,
            title,
            container(controls)
                .width(Length::Fill)
                .align_right(Length::Fill)
        ]
        .align_y(Alignment::Center)
        .spacing(0),
    )
    .width(Length::Fill)
    .height(Length::Fixed(TITLE_BAR_HEIGHT))
    .padding([0, 6])
    .align_y(iced::alignment::Vertical::Center)
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(colors.surface)
            .color(colors.text)
    })
    .into()
}

pub fn app_shell<'a, Message>(
    title_bar: impl Into<Element<'a, Message>>,
    workspace: impl Into<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: 'a,
{
    container(column![title_bar.into(), workspace.into()])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.background)
                .color(colors.text)
        })
        .into()
}

pub(crate) fn section_heading<'a>(
    title: &'a str,
    trailing: Option<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message> {
    let mut content = row![
        text(title)
            .size(12)
            .font(ui_font(font::Weight::Bold))
            .color(colors.muted)
    ]
    .align_y(Alignment::Center)
    .spacing(8);
    if let Some(trailing) = trailing {
        content = content.push(space().width(Length::Fill)).push(trailing);
    }
    container(content)
        .height(Length::Fixed(UI_METRICS.selection_height))
        .padding([0.0, UI_METRICS.selection_padding_x])
        .align_y(iced::alignment::Vertical::Center)
        .into()
}
