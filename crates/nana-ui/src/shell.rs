use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Font, Length, font};

use crate::geometry::TITLE_BAR_HEIGHT;
use crate::theme::{Colors, ThemeMode};
use crate::widgets::{ButtonKind, button_style};
use crate::workspace_demo::{LayoutPreset, Message, WorkspaceState};

pub(crate) fn title_bar<'a>(state: &WorkspaceState, colors: Colors) -> Element<'a, Message> {
    let sidebar_toggle = button(text("☰").size(15).color(colors.muted))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(28.0))
        .padding(0)
        .on_press(Message::Workspace(
            crate::workspace::WorkspaceAction::ToggleRegion(crate::layout::RegionId::Resources),
        ))
        .style(button_style(colors, ButtonKind::Ghost));
    let mut workspace_switcher = row![sidebar_toggle].spacing(4).align_y(Alignment::Center);
    for (preset, label) in [
        (LayoutPreset::Code, "Code"),
        (LayoutPreset::Github, "Github"),
        (LayoutPreset::Live2D, "Live2D"),
    ] {
        workspace_switcher = workspace_switcher.push(
            button(text(label).size(11))
                .padding([5, 8])
                .on_press(Message::SelectLayout(preset))
                .style(button_style(
                    colors,
                    if state.layout_preset() == preset {
                        ButtonKind::Selected
                    } else {
                        ButtonKind::Text
                    },
                )),
        );
    }
    app_title_bar(
        state.layout_title(),
        "工作区",
        state.theme_mode(),
        Message::ToggleTheme,
        Some(workspace_switcher.into()),
        colors,
    )
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
    let theme_glyph = match theme {
        ThemeMode::Dark => "☀",
        ThemeMode::Light => "☾",
    };
    let leading =
        container(leading_action.unwrap_or_else(|| space().width(Length::Fixed(28.0)).into()))
            .width(Length::Fill)
            .align_left(Length::Fill);

    let title = container(
        text(title)
            .size(13)
            .font(Font {
                weight: font::Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(colors.text),
    )
    .width(Length::Fixed(140.0))
    .center_x(Length::Fill);

    let controls = row![
        text(context).size(11).color(colors.muted),
        button(text(theme_glyph).size(14))
            .on_press(toggle_theme)
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(28.0))
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
    let mut content = row![text(title).size(12).color(colors.muted)]
        .align_y(Alignment::Center)
        .spacing(8);
    if let Some(trailing) = trailing {
        content = content.push(space().width(Length::Fill)).push(trailing);
    }
    container(content).padding([10, 12]).into()
}
