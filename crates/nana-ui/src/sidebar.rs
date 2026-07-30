use std::borrow::Cow;

use iced::widget::{button, column, container, row, scrollable, space, stack, text, tooltip};
use iced::{
    Alignment, Animation, Border, Color, Element, Length, Padding, Shadow, Subscription, Theme,
    font,
};

use crate::icons::{Icon, disclosure_icon, icon};
use crate::theme::{Colors, ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{scrollable_style, vertical_scrollbar};

const FRAME_PADDING_TOP: f32 = 10.0;
const FRAME_PADDING_RIGHT: f32 = 8.0;
const FRAME_PADDING_BOTTOM: f32 = 10.0;
const FRAME_PADDING_LEFT: f32 = 12.0;
const FRAME_GAP: f32 = 14.0;
const ROW_HEIGHT: f32 = UI_METRICS.navigation_row_height;
const ROW_PADDING_LEFT: f32 = 8.0;
const ROW_ICON_SLOT_WIDTH: f32 = 16.0;
const SECTION_ANIMATION_DURATION: iced::time::Duration = iced::time::Duration::from_millis(160);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SidebarRowState {
    #[default]
    Idle,
    Active,
    AncestorActive,
    Disabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SidebarRowTone {
    #[default]
    Default,
    Warning,
    Error,
}

/// Persistent expansion state and frame subscription for a sidebar section.
#[derive(Debug, Clone)]
pub struct SidebarSectionState {
    expanded: bool,
    expansion: Animation<bool>,
}

impl SidebarSectionState {
    pub fn new(expanded: bool) -> Self {
        Self {
            expanded,
            expansion: Animation::new(expanded)
                .duration(SECTION_ANIMATION_DURATION)
                .easing(iced::animation::Easing::EaseOutCubic),
        }
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn set_expanded(&mut self, expanded: bool) -> bool {
        self.set_expanded_at(expanded, iced::time::Instant::now())
    }

    pub fn toggle(&mut self) -> bool {
        self.set_expanded(!self.expanded)
    }

    pub fn is_animating(&self) -> bool {
        self.expansion.is_animating(iced::time::Instant::now())
    }

    pub fn expansion(&self) -> f32 {
        self.expansion_at(iced::time::Instant::now())
    }

    pub fn subscription(&self) -> Subscription<iced::time::Instant> {
        if self.is_animating() {
            iced::window::frames()
        } else {
            Subscription::none()
        }
    }

    fn set_expanded_at(&mut self, expanded: bool, at: iced::time::Instant) -> bool {
        if self.expanded == expanded {
            return false;
        }
        self.expanded = expanded;
        self.expansion.go_mut(expanded, at);
        true
    }

    fn expansion_at(&self, at: iced::time::Instant) -> f32 {
        self.expansion.interpolate(0.0, 1.0, at)
    }
}

impl Default for SidebarSectionState {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Three-part sidebar surface with a scrolling body and fixed outer slots.
pub struct SidebarFrame<'a, Message> {
    top: Option<Element<'a, Message>>,
    body: Element<'a, Message>,
    footer: Option<Element<'a, Message>>,
    gap: f32,
}

impl<'a, Message> SidebarFrame<'a, Message>
where
    Message: 'a,
{
    pub fn new(body: impl Into<Element<'a, Message>>) -> Self {
        Self {
            top: None,
            body: body.into(),
            footer: None,
            gap: FRAME_GAP,
        }
    }

    pub fn top(mut self, top: impl Into<Element<'a, Message>>) -> Self {
        self.top = Some(top.into());
        self
    }

    pub fn footer(mut self, footer: impl Into<Element<'a, Message>>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn view(self, colors: Colors) -> Element<'a, Message> {
        let mut content = column![].spacing(self.gap);
        if let Some(top) = self.top {
            content = content.push(top);
        }
        content = content.push(
            scrollable(self.body)
                .direction(vertical_scrollbar())
                .style(scrollable_style(colors))
                .width(Length::Fill)
                .height(Length::Fill),
        );
        if let Some(footer) = self.footer {
            content = content.push(footer);
        }

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: FRAME_PADDING_TOP,
                right: FRAME_PADDING_RIGHT,
                bottom: FRAME_PADDING_BOTTOM,
                left: FRAME_PADDING_LEFT,
            })
            .clip(true)
            .into()
    }
}

/// A compact sidebar navigation row.
pub struct SidebarRow<'a, Message> {
    label: Cow<'a, str>,
    leading: Option<Element<'a, Message>>,
    trailing: Option<Element<'a, Message>>,
    depth: u16,
    height: f32,
    state: SidebarRowState,
    tone: SidebarRowTone,
    on_select: Option<Message>,
    disclosure: Option<(bool, Message)>,
}

impl<'a, Message> SidebarRow<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            label: label.into(),
            leading: None,
            trailing: None,
            depth: 0,
            height: ROW_HEIGHT,
            state: SidebarRowState::Idle,
            tone: SidebarRowTone::Default,
            on_select: None,
            disclosure: None,
        }
    }

    pub fn leading(mut self, leading: impl Into<Element<'a, Message>>) -> Self {
        self.leading = Some(leading.into());
        self
    }

    pub fn trailing(mut self, trailing: impl Into<Element<'a, Message>>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(ROW_HEIGHT);
        self
    }

    pub fn state(mut self, state: SidebarRowState) -> Self {
        self.state = state;
        self
    }

    pub fn tone(mut self, tone: SidebarRowTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn on_select(mut self, message: Message) -> Self {
        self.on_select = Some(message);
        self
    }

    pub fn disclosure(mut self, expanded: bool, on_toggle: Message) -> Self {
        self.disclosure = Some((expanded, on_toggle));
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let disabled = self.state == SidebarRowState::Disabled;
        let depth_inset = ROW_PADDING_LEFT + f32::from(self.depth) * 14.0;
        let has_disclosure = self.disclosure.is_some();
        let mut content = row![]
            .width(Length::Fill)
            .spacing(6)
            .align_y(Alignment::Center);
        if has_disclosure {
            content = content.push(space().width(Length::Fixed(14.0)));
        }
        if let Some(leading) = self.leading {
            content = content.push(
                container(leading)
                    .width(Length::Fixed(ROW_ICON_SLOT_WIDTH))
                    .align_x(iced::alignment::Horizontal::Center),
            );
        }
        content = content.push(
            text(self.label)
                .size(13)
                .font(ui_font(if self.state == SidebarRowState::AncestorActive {
                    font::Weight::Semibold
                } else {
                    font::Weight::Medium
                }))
                .width(Length::Fill)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End),
        );
        if let Some(trailing) = self.trailing {
            content = content.push(trailing);
        }

        let select = button(content)
            .width(Length::Fill)
            .height(Length::Fixed(self.height))
            .padding(Padding {
                top: 0.0,
                right: 8.0,
                bottom: 0.0,
                left: depth_inset,
            })
            .align_x(iced::alignment::Horizontal::Left)
            .on_press_maybe((!disabled).then_some(self.on_select).flatten())
            .style(sidebar_row_style(
                colors,
                self.state,
                self.tone,
                tokens.metrics.radius_sm,
            ));

        let mut layers = stack![select];
        if let Some((expanded, on_toggle)) = self.disclosure {
            let disclosure = button(disclosure_icon(
                if expanded { 1.0 } else { 0.0 },
                10.0,
                colors.muted,
            ))
            .width(Length::Fixed(14.0))
            .height(Length::Fixed(20.0))
            .padding(0)
            .on_press_maybe((!disabled).then_some(on_toggle))
            .style(disclosure_style(colors, tokens.metrics.radius_xs));
            layers = layers.push(
                container(disclosure)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(Padding {
                        top: 0.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: depth_inset,
                    })
                    .align_left(Length::Fill)
                    .center_y(Length::Fill),
            );
        }
        container(layers)
            .width(Length::Fill)
            .height(Length::Fixed(self.height))
            .into()
    }
}

/// A titled, optionally collapsible sidebar group.
pub struct SidebarSection<'a, Message> {
    title: Cow<'a, str>,
    count: Option<usize>,
    expanded: bool,
    on_toggle: Option<Message>,
    empty_text: Option<Cow<'a, str>>,
    children: Vec<Element<'a, Message>>,
    animation_progress: Option<f32>,
}

impl<'a, Message> SidebarSection<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(title: impl Into<Cow<'a, str>>) -> Self {
        Self {
            title: title.into(),
            count: None,
            expanded: true,
            on_toggle: None,
            empty_text: None,
            children: Vec::new(),
            animation_progress: None,
        }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn on_toggle(mut self, message: Message) -> Self {
        self.on_toggle = Some(message);
        self
    }

    pub fn empty_text(mut self, empty_text: impl Into<Cow<'a, str>>) -> Self {
        self.empty_text = Some(empty_text.into());
        self
    }

    pub fn animation_progress(mut self, progress: f32) -> Self {
        self.animation_progress = Some(progress.clamp(0.0, 1.0));
        self
    }

    pub fn push(mut self, child: impl Into<Element<'a, Message>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let collapsible = self.on_toggle.is_some();
        let expansion = if collapsible {
            self.animation_progress
                .unwrap_or(if self.expanded { 1.0 } else { 0.0 })
        } else {
            1.0
        };
        let mut heading = row![].spacing(5).align_y(Alignment::Center);
        if collapsible {
            heading = heading.push(disclosure_icon(expansion, 12.0, colors.faint));
        }
        heading = heading.push(
            tracked_label(
                &self.title.to_uppercase(),
                11.0,
                font::Weight::Bold,
                0.5,
                colors.faint,
            )
            .width(Length::Fill),
        );
        if let Some(count) = self.count {
            heading = heading.push(text(count).size(11).color(colors.faint));
        }

        let header = button(heading)
            .width(Length::Fill)
            .height(Length::Fixed(ROW_HEIGHT))
            .padding([0.0, ROW_PADDING_LEFT])
            .align_x(iced::alignment::Horizontal::Left)
            .on_press_maybe(self.on_toggle)
            .style(section_header_style(
                colors,
                collapsible,
                tokens.metrics.radius_sm,
            ));

        let mut section = column![header];
        if expansion > 0.0 {
            let child_count = self.children.len();
            let mut children = column![].spacing(1);
            let content_height = if child_count == 0 {
                if let Some(empty_text) = self.empty_text {
                    children = children.push(
                        container(text(empty_text).size(12).color(colors.faint)).padding([6, 8]),
                    );
                    30.0
                } else {
                    0.0
                }
            } else {
                for child in self.children {
                    children = children.push(child);
                }
                child_count as f32 * ROW_HEIGHT + child_count.saturating_sub(1) as f32
            };
            if content_height > 0.0 {
                section = section.push(
                    container(children)
                        .width(Length::Fill)
                        .height(Length::Fixed(content_height * expansion))
                        .clip(true),
                );
            }
        }
        section.width(Length::Fill).into()
    }
}

/// Fixed footer content for a [`SidebarFrame`].
pub struct SidebarFooter<'a, Message> {
    children: Vec<Element<'a, Message>>,
}

impl<'a, Message> SidebarFooter<'a, Message>
where
    Message: 'a,
{
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn push(mut self, child: impl Into<Element<'a, Message>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn view(self, _colors: Colors) -> Element<'a, Message> {
        let mut content = row![].spacing(2).align_y(Alignment::Center);
        for child in self.children {
            content = content.push(child);
        }
        content.width(Length::Fill).into()
    }
}

impl<'a, Message: 'a> Default for SidebarFooter<'a, Message> {
    fn default() -> Self {
        Self::new()
    }
}

/// Lilia-style icon action for the fixed sidebar footer.
pub struct SidebarFooterButton<'a, Message> {
    label: Cow<'a, str>,
    icon: Icon,
    selected: bool,
    on_press: Option<Message>,
}

impl<'a, Message> SidebarFooterButton<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>, icon: Icon) -> Self {
        Self {
            label: label.into(),
            icon,
            selected: false,
            on_press: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let icon_color = if self.selected {
            colors.text
        } else {
            colors.muted.scale_alpha(0.44)
        };
        let action = button(icon(self.icon, 14.0, icon_color))
            .width(Length::Fixed(UI_METRICS.sidebar_footer_button_size))
            .height(Length::Fixed(UI_METRICS.sidebar_footer_button_size))
            .padding(0)
            .on_press_maybe(self.on_press)
            .style(sidebar_footer_button_style(
                colors,
                self.selected,
                tokens.metrics.radius_sm,
            ));
        tooltip(
            container(action)
                .width(Length::Fixed(UI_METRICS.control_height))
                .align_x(iced::alignment::Horizontal::Center),
            container(text(self.label).size(11)).padding([4, 7]),
            tooltip::Position::Top,
        )
        .gap(6)
        .into()
    }
}

fn sidebar_row_style(
    colors: Colors,
    state: SidebarRowState,
    tone: SidebarRowTone,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_theme, status| {
        let disabled = state == SidebarRowState::Disabled || status == button::Status::Disabled;
        let foreground = match tone {
            SidebarRowTone::Default
                if matches!(
                    state,
                    SidebarRowState::Active | SidebarRowState::AncestorActive
                ) =>
            {
                colors.text
            }
            SidebarRowTone::Default if disabled => colors.faint,
            SidebarRowTone::Default => colors.muted,
            SidebarRowTone::Warning => colors.warning,
            SidebarRowTone::Error => colors.danger,
        };
        let background = match status {
            button::Status::Hovered if state == SidebarRowState::Active => colors.selected_hover,
            button::Status::Hovered => colors.hover,
            button::Status::Pressed if state == SidebarRowState::Active => colors.selected_pressed,
            button::Status::Pressed => colors.active,
            button::Status::Active if state == SidebarRowState::Active => colors.selected,
            button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = if disabled {
            Color {
                a: 0.68,
                ..foreground
            }
        } else {
            foreground
        };
        style.border = Border::default().rounded(radius);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

fn disclosure_style(
    colors: Colors,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered => colors.hover,
            button::Status::Pressed => colors.active,
            button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = if status == button::Status::Disabled {
            colors.faint
        } else {
            colors.muted
        };
        style.border = Border::default().rounded(radius);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

fn section_header_style(
    colors: Colors,
    interactive: bool,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_theme, status| {
        let background = if interactive {
            match status {
                button::Status::Hovered => colors.hover,
                button::Status::Pressed => colors.active,
                button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
            }
        } else {
            Color::TRANSPARENT
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = colors.faint;
        style.border = Border::default().rounded(radius);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

fn sidebar_footer_button_style(
    colors: Colors,
    selected: bool,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered if selected => colors.selected_hover,
            button::Status::Hovered => colors.hover,
            button::Status::Pressed if selected => colors.selected_pressed,
            button::Status::Pressed => colors.active,
            button::Status::Active if selected => colors.selected,
            button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
        };
        let foreground = if status == button::Status::Disabled {
            colors.faint.scale_alpha(0.45)
        } else if selected || matches!(status, button::Status::Hovered | button::Status::Pressed) {
            colors.text
        } else {
            colors.muted.scale_alpha(0.44)
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = foreground;
        style.border = Border::default().rounded(radius);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

#[cfg(test)]
mod tests {
    use iced::Theme;
    use iced::widget::button;

    use super::{
        SidebarRowState, SidebarRowTone, SidebarSectionState, section_header_style,
        sidebar_row_style,
    };
    use crate::theme::{ThemeMode, UI_METRICS};

    #[test]
    fn section_state_animates_and_reverses_expansion() {
        let started = iced::time::Instant::now();
        let mut state = SidebarSectionState::new(true);

        assert!(state.set_expanded_at(false, started));
        assert!(!state.expanded());
        let middle = state.expansion_at(started + iced::time::Duration::from_millis(80));
        assert!(middle > 0.0 && middle < 1.0);
        assert_eq!(
            state.expansion_at(started + iced::time::Duration::from_millis(200)),
            0.0
        );

        assert!(state.set_expanded_at(true, started + iced::time::Duration::from_millis(80)));
        assert!(state.expanded());
        assert_eq!(
            state.expansion_at(started + iced::time::Duration::from_millis(280)),
            1.0
        );
    }

    #[test]
    fn section_headers_and_rows_share_the_control_radius() {
        let colors = ThemeMode::Dark.colors();
        let row = sidebar_row_style(
            colors,
            SidebarRowState::Active,
            SidebarRowTone::Default,
            UI_METRICS.radius_sm,
        )(&Theme::Dark, button::Status::Active);
        let section = section_header_style(colors, true, UI_METRICS.radius_sm)(
            &Theme::Dark,
            button::Status::Hovered,
        );

        assert_eq!(row.border.radius, section.border.radius);
    }
}
