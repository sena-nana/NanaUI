use std::borrow::Cow;
use std::rc::Rc;

use iced::widget::text::LineHeight;
use iced::widget::{Stack, button, column, container, mouse_area, pin, row, text, text_input};
use iced::{Alignment, Element, Length, Pixels, Point, Size};

use crate::components::ControlSize;
use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{menu_item_style, menu_surface_style, text_input_style};

/// A selectable row shared by anchored action menus and context menus.
pub struct ActionMenuItem<'a, Message> {
    label: Cow<'a, str>,
    hint: Option<Cow<'a, str>>,
    leading: Option<Icon>,
    on_press: Option<Message>,
    size: ControlSize,
    active: bool,
    danger: bool,
    disabled: bool,
}

impl<'a, Message> ActionMenuItem<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            leading: None,
            on_press: None,
            size: ControlSize::Small,
            active: false,
            danger: false,
            disabled: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn leading(mut self, leading: Icon) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut content = row![].spacing(8).align_y(Alignment::Center);
        if let Some(leading) = self.leading {
            content = content.push(icon(
                leading,
                self.size.icon_size(),
                if self.danger {
                    colors.danger
                } else {
                    colors.muted
                },
            ));
        }
        content = content.push(
            text(self.label)
                .size(self.size.text_size())
                .line_height(LineHeight::Absolute(Pixels(self.size.line_height())))
                .font(ui_font(iced::font::Weight::Medium))
                .width(Length::Fill),
        );
        if let Some(hint) = self.hint {
            content = content.push(
                text(hint)
                    .size(11)
                    .line_height(LineHeight::Absolute(Pixels(self.size.line_height())))
                    .color(colors.muted),
            );
        }
        button(content)
            .width(Length::Fill)
            .height(Length::Fixed(self.size.height_in(tokens.metrics)))
            .padding([0.0, self.size.padding_x()])
            .align_x(iced::alignment::Horizontal::Left)
            .on_press_maybe((!self.disabled).then_some(self.on_press).flatten())
            .style(menu_item_style(tokens, self.danger, self.active))
            .into()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnchoredMenuPlacement {
    TopStart,
    TopEnd,
    #[default]
    BottomStart,
    BottomEnd,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnchoredMenuPosition {
    pub anchor: Point,
    pub placement: AnchoredMenuPlacement,
    pub offset: f32,
}

impl AnchoredMenuPosition {
    pub const fn new(anchor: Point) -> Self {
        Self {
            anchor,
            placement: AnchoredMenuPlacement::BottomStart,
            offset: 0.0,
        }
    }

    pub const fn placement(mut self, placement: AnchoredMenuPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset.max(0.0);
        self
    }

    pub fn resolve(self, menu: Size, viewport: Size) -> Point {
        let mut point = match self.placement {
            AnchoredMenuPlacement::TopStart => {
                Point::new(self.anchor.x, self.anchor.y - menu.height - self.offset)
            }
            AnchoredMenuPlacement::TopEnd => Point::new(
                self.anchor.x - menu.width,
                self.anchor.y - menu.height - self.offset,
            ),
            AnchoredMenuPlacement::BottomStart => {
                Point::new(self.anchor.x, self.anchor.y + self.offset)
            }
            AnchoredMenuPlacement::BottomEnd => {
                Point::new(self.anchor.x - menu.width, self.anchor.y + self.offset)
            }
        };
        point.x = point.x.clamp(0.0, (viewport.width - menu.width).max(0.0));
        point.y = point.y.clamp(0.0, (viewport.height - menu.height).max(0.0));
        point
    }
}

/// A viewport-level menu surface pinned to a logical anchor point.
///
/// The caller owns open state and supplies distinct outside-dismiss and
/// inside-interaction messages, so the menu composes safely in [`OverlayHost`].
pub struct AnchoredActionMenu<'a, Message> {
    content: Element<'a, Message>,
    position: AnchoredMenuPosition,
    viewport: Size,
    menu_size: Size,
    on_dismiss: Message,
    on_interaction: Message,
}

impl<'a, Message> AnchoredActionMenu<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        position: AnchoredMenuPosition,
        viewport: Size,
        on_dismiss: Message,
        on_interaction: Message,
    ) -> Self {
        Self {
            content: content.into(),
            position,
            viewport,
            menu_size: Size::new(200.0, 240.0),
            on_dismiss,
            on_interaction,
        }
    }

    pub fn menu_size(mut self, width: f32, height: f32) -> Self {
        self.menu_size = Size::new(width.max(120.0), height.max(32.0));
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let point = self.position.resolve(self.menu_size, self.viewport);
        let surface = mouse_area(
            container(self.content)
                .width(Length::Fixed(self.menu_size.width))
                .height(Length::Fixed(self.menu_size.height))
                .padding(4)
                .style(menu_surface_style(tokens)),
        )
        .on_press(self.on_interaction);
        mouse_area(
            pin(surface)
                .position(point)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(self.on_dismiss)
        .into()
    }
}

/// A stack host whose first element is application content and whose remaining
/// elements are transient surfaces in visual and event order.
pub struct OverlayHost<'a, Message> {
    layers: Vec<Element<'a, Message>>,
}

impl<'a, Message> OverlayHost<'a, Message>
where
    Message: 'a,
{
    pub fn new(base: impl Into<Element<'a, Message>>) -> Self {
        Self {
            layers: vec![base.into()],
        }
    }

    pub fn push(mut self, overlay: impl Into<Element<'a, Message>>) -> Self {
        self.layers.push(overlay.into());
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        Stack::with_children(self.layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuItem<'a, T> {
    pub value: T,
    pub label: Cow<'a, str>,
    pub hint: Option<Cow<'a, str>>,
    pub keywords: Vec<Cow<'a, str>>,
    pub confirm_label: Option<Cow<'a, str>>,
    pub icon: Option<Icon>,
    pub children: Vec<ContextMenuItem<'a, T>>,
    pub disabled: bool,
    pub danger: bool,
}

impl<'a, T> ContextMenuItem<'a, T> {
    pub fn new(value: T, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            value,
            label: label.into(),
            hint: None,
            keywords: Vec::new(),
            confirm_label: None,
            icon: None,
            children: Vec::new(),
            disabled: false,
            danger: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<Cow<'a, str>>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn confirm_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.confirm_label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuEvent<T> {
    Search(String),
    OpenSubmenu(Vec<usize>),
    Select(T),
    Dismiss,
    Interaction,
}

/// A searchable context-menu renderer with click-opened submenus and shared
/// destructive confirmation presentation.
pub struct ContextMenuHost<'a, T, Message> {
    items: &'a [ContextMenuItem<'a, T>],
    position: AnchoredMenuPosition,
    viewport: Size,
    query: &'a str,
    active_path: &'a [usize],
    pending: Option<&'a T>,
    searchable: bool,
    on_event: Rc<dyn Fn(ContextMenuEvent<T>) -> Message + 'a>,
    tokens: ThemeTokens,
}

impl<'a, T, Message> ContextMenuHost<'a, T, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        items: &'a [ContextMenuItem<'a, T>],
        position: AnchoredMenuPosition,
        viewport: Size,
        on_event: impl Fn(ContextMenuEvent<T>) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            items,
            position,
            viewport,
            query: "",
            active_path: &[],
            pending: None,
            searchable: false,
            on_event: Rc::new(on_event),
            tokens: theme.into(),
        }
    }

    pub fn search(mut self, query: &'a str, searchable: bool) -> Self {
        self.query = query;
        self.searchable = searchable;
        self
    }

    pub fn active_path(mut self, path: &'a [usize]) -> Self {
        self.active_path = path;
        self
    }

    pub fn pending(mut self, value: Option<&'a T>) -> Self {
        self.pending = value;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let mut panels: Vec<Element<'a, Message>> = Vec::new();
        if self.searchable && !self.query.trim().is_empty() {
            let matches = collect_matches(self.items, self.query);
            panels.push(self.panel(
                matches.into_iter().map(|(_, item)| (Vec::new(), item)),
                true,
            ));
        } else {
            panels.push(
                self.panel(
                    self.items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| (vec![index], item)),
                    false,
                ),
            );
            let mut items = self.items;
            let mut path = Vec::new();
            for &index in self.active_path {
                let Some(item) = items.get(index) else {
                    break;
                };
                if item.children.is_empty() {
                    break;
                }
                path.push(index);
                let parent = path.clone();
                panels.push(self.panel(
                    item.children.iter().enumerate().map(move |(child, item)| {
                        let mut item_path = parent.clone();
                        item_path.push(child);
                        (item_path, item)
                    }),
                    false,
                ));
                items = &item.children;
            }
        }

        let panel_count = panels.len() as f32;
        let search_size = ControlSize::Small;
        let search_height = if self.searchable {
            search_size.height_in(self.tokens.metrics) + 10.0
        } else {
            0.0
        };
        let mut content = column![].spacing(4);
        if self.searchable {
            content = content.push(
                text_input("搜索操作", self.query)
                    .on_input({
                        let on_event = Rc::clone(&self.on_event);
                        move |query| on_event(ContextMenuEvent::Search(query))
                    })
                    .padding([
                        search_size.vertical_padding(self.tokens.metrics),
                        search_size.padding_x(),
                    ])
                    .size(search_size.text_size())
                    .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                        search_size.line_height(),
                    )))
                    .style(text_input_style(self.tokens, false)),
            );
        }
        content = content.push(row(panels).spacing(4));
        let on_dismiss = (self.on_event)(ContextMenuEvent::Dismiss);
        let on_interaction = (self.on_event)(ContextMenuEvent::Interaction);
        AnchoredActionMenu::new(
            content,
            self.position,
            self.viewport,
            on_dismiss,
            on_interaction,
        )
        .menu_size(200.0 * panel_count, 244.0 + search_height)
        .view(self.tokens)
    }

    fn panel(
        &self,
        items: impl IntoIterator<Item = (Vec<usize>, &'a ContextMenuItem<'a, T>)>,
        flattened: bool,
    ) -> Element<'a, Message> {
        let mut panel = column![].spacing(1).width(Length::Fixed(192.0));
        for (path, item) in items {
            let pending = self.pending == Some(&item.value);
            let label = if pending {
                item.confirm_label
                    .clone()
                    .unwrap_or_else(|| item.label.clone())
            } else {
                item.label.clone()
            };
            let event = if !flattened && !item.children.is_empty() {
                ContextMenuEvent::OpenSubmenu(path)
            } else {
                ContextMenuEvent::Select(item.value.clone())
            };
            let mut menu_item = ActionMenuItem::new(label)
                .active(pending)
                .danger(item.danger || pending)
                .disabled(item.disabled)
                .on_press((self.on_event)(event));
            if let Some(icon) = item.icon {
                menu_item = menu_item.leading(icon);
            }
            if !item.children.is_empty() {
                menu_item = menu_item.hint("›");
            } else if let Some(hint) = &item.hint {
                menu_item = menu_item.hint(hint.clone());
            }
            panel = panel.push(menu_item.view(self.tokens));
        }
        container(panel).height(Length::Fill).into()
    }
}

fn collect_matches<'a, T>(
    items: &'a [ContextMenuItem<'a, T>],
    query: &str,
) -> Vec<(Vec<usize>, &'a ContextMenuItem<'a, T>)> {
    fn visit<'a, T>(
        items: &'a [ContextMenuItem<'a, T>],
        query: &str,
        path: &mut Vec<usize>,
        matches: &mut Vec<(Vec<usize>, &'a ContextMenuItem<'a, T>)>,
    ) {
        for (index, item) in items.iter().enumerate() {
            path.push(index);
            if item.children.is_empty()
                && std::iter::once(item.label.as_ref())
                    .chain(item.keywords.iter().map(AsRef::as_ref))
                    .any(|candidate| candidate.to_lowercase().contains(query))
            {
                matches.push((path.clone(), item));
            }
            visit(&item.children, query, path, matches);
            path.pop();
        }
    }

    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    visit(items, &query, &mut Vec::new(), &mut matches);
    matches
}

#[cfg(test)]
mod tests {
    use super::{
        ActionMenuItem, AnchoredMenuPlacement, AnchoredMenuPosition, ContextMenuItem, ControlSize,
        collect_matches,
    };
    use iced::{Point, Size};

    #[test]
    fn anchored_menu_position_stays_inside_the_viewport() {
        let position = AnchoredMenuPosition::new(Point::new(395.0, 295.0))
            .placement(AnchoredMenuPlacement::BottomEnd)
            .resolve(Size::new(180.0, 120.0), Size::new(400.0, 300.0));
        assert_eq!(position, Point::new(215.0, 180.0));
    }

    #[test]
    fn context_search_finds_nested_leaf_keywords_only() {
        let items = [ContextMenuItem::new("file", "文件").children([
            ContextMenuItem::new("rename", "重命名").keywords(["edit"]),
            ContextMenuItem::new("remove", "删除").keywords(["danger"]),
        ])];
        let matches = collect_matches(&items, "danger");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, vec![0, 1]);
        assert_eq!(matches[0].1.value, "remove");
    }

    #[test]
    fn action_menu_items_default_to_the_small_density_tier() {
        assert_eq!(ActionMenuItem::<()>::new("操作").size, ControlSize::Small);
    }
}
