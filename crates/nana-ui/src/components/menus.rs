use std::borrow::Cow;
use std::rc::Rc;

use iced::widget::text::LineHeight;
use iced::widget::{
    Stack, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Pixels, Point, Size};

use crate::absolute::Absolute;
use crate::components::ControlSize;
use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{
    menu_item_style, menu_surface_style, scrollable_style, text_input_style, vertical_scrollbar,
};

const MENU_PANEL_WIDTH: f32 = 192.0;
const MENU_PANEL_SPACING: f32 = 4.0;
const MENU_ITEM_SPACING: f32 = 1.0;
const MENU_CONTENT_SPACING: f32 = 4.0;
const MENU_SURFACE_PADDING: f32 = 4.0;
const MENU_MIN_WIDTH: f32 = 120.0;
const MENU_MIN_HEIGHT: f32 = 32.0;

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
        self.menu_size = Size::new(width.max(MENU_MIN_WIDTH), height.max(MENU_MIN_HEIGHT));
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let point = self.position.resolve(self.menu_size, self.viewport);
        let surface = mouse_area(
            container(self.content)
                .width(Length::Fixed(self.menu_size.width))
                .height(Length::Fixed(self.menu_size.height))
                .padding(MENU_SURFACE_PADDING)
                .style(menu_surface_style(tokens)),
        )
        .on_press(self.on_interaction);
        mouse_area(Absolute::new(surface, point))
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
        let mut panel_count = 1;
        let mut max_panel_items = self.items.len();
        if self.searchable && !self.query.trim().is_empty() {
            let matches = collect_matches(self.items, self.query);
            max_panel_items = matches.len();
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
                panel_count += 1;
                max_panel_items = max_panel_items.max(item.children.len());
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

        let search_size = ControlSize::Small;
        let menu_size = context_menu_size(
            panel_count,
            max_panel_items,
            self.searchable,
            self.tokens,
            self.viewport,
        );
        let search_height = if self.searchable {
            search_size.height_in(self.tokens.metrics)
        } else {
            0.0
        };
        let content_spacing = if self.searchable {
            MENU_CONTENT_SPACING
        } else {
            0.0
        };
        // Fixed list height — avoid Length::Fill collapsing when an ancestor
        // (historically iced::pin) clamps the menu surface near a corner.
        let list_height = (menu_size.height
            - MENU_SURFACE_PADDING * 2.0
            - search_height
            - content_spacing)
            .max(0.0);
        let mut content = column![].spacing(MENU_CONTENT_SPACING);
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
        content = content.push(
            scrollable(row(panels).spacing(MENU_PANEL_SPACING))
                .direction(vertical_scrollbar())
                .style(scrollable_style(self.tokens.colors))
                .height(Length::Fixed(list_height)),
        );
        let on_dismiss = (self.on_event)(ContextMenuEvent::Dismiss);
        let on_interaction = (self.on_event)(ContextMenuEvent::Interaction);
        AnchoredActionMenu::new(
            content,
            self.position,
            self.viewport,
            on_dismiss,
            on_interaction,
        )
        .menu_size(menu_size.width, menu_size.height)
        .view(self.tokens)
    }

    fn panel(
        &self,
        items: impl IntoIterator<Item = (Vec<usize>, &'a ContextMenuItem<'a, T>)>,
        flattened: bool,
    ) -> Element<'a, Message> {
        let mut panel = column![]
            .spacing(MENU_ITEM_SPACING)
            .width(Length::Fixed(MENU_PANEL_WIDTH));
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
        container(panel).into()
    }
}

fn context_menu_size(
    panel_count: usize,
    max_panel_items: usize,
    searchable: bool,
    tokens: ThemeTokens,
    viewport: Size,
) -> Size {
    let panel_count = panel_count.max(1);
    let width = MENU_SURFACE_PADDING * 2.0
        + panel_count as f32 * MENU_PANEL_WIDTH
        + panel_count.saturating_sub(1) as f32 * MENU_PANEL_SPACING;
    let item_height = ControlSize::Small.height_in(tokens.metrics);
    let panel_height = max_panel_items as f32 * item_height
        + max_panel_items.saturating_sub(1) as f32 * MENU_ITEM_SPACING;
    let search_height = if searchable { item_height } else { 0.0 };
    let content_spacing = if searchable {
        MENU_CONTENT_SPACING
    } else {
        0.0
    };
    let intrinsic_height =
        MENU_SURFACE_PADDING * 2.0 + search_height + content_spacing + panel_height;
    let max_height = viewport.height.max(MENU_MIN_HEIGHT);
    Size::new(width, intrinsic_height.min(max_height).max(MENU_MIN_HEIGHT))
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
        collect_matches, context_menu_size,
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

    #[test]
    fn context_menu_size_matches_visible_panel_content() {
        let tokens = crate::theme::ThemeMode::Dark.colors().into();
        let size = context_menu_size(1, 1, false, tokens, Size::new(800.0, 600.0));
        let item_height = ControlSize::Small.height_in(tokens.metrics);

        assert_eq!(size.width, 200.0);
        assert_eq!(size.height, item_height + 8.0);
    }

    #[test]
    fn context_menu_size_accounts_for_multiple_panels_and_search() {
        let tokens = crate::theme::ThemeMode::Dark.colors().into();
        let size = context_menu_size(2, 3, true, tokens, Size::new(800.0, 600.0));
        let item_height = ControlSize::Small.height_in(tokens.metrics);

        assert_eq!(size.width, 396.0);
        assert_eq!(
            size.height,
            8.0 + item_height + 4.0 + item_height * 3.0 + 2.0
        );
    }

    #[test]
    fn context_menu_size_caps_long_panels_to_the_viewport() {
        let tokens = crate::theme::ThemeMode::Dark.colors().into();
        let viewport = Size::new(320.0, 180.0);
        let size = context_menu_size(1, 100, true, tokens, viewport);

        assert_eq!(size.height, viewport.height);
        let position = AnchoredMenuPosition::new(Point::new(310.0, 170.0))
            .placement(AnchoredMenuPlacement::BottomEnd)
            .resolve(size, viewport);
        assert_eq!(position, Point::new(110.0, 0.0));
    }
}
