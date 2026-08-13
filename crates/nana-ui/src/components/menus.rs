use std::borrow::Cow;
use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::text::LineHeight;
use iced::widget::{
    Stack, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{
    Alignment, Element, Event, Length, Pixels, Point, Rectangle, Size, Theme, Vector, keyboard,
    touch,
};

use crate::absolute::Absolute;
use crate::components::ControlSize;
use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{
    menu_item_style, menu_surface_style, scrollable_style, text_input_style, vertical_scrollbar,
};

const MENU_PANEL_WIDTH: f32 = 192.0;
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

pub use nana_ui_core::AnchoredMenuPlacement;

/// A logical window-space anchor captured by [`ContextMenuTrigger`].
///
/// The position is intentionally opaque so pointer-triggered context menus use
/// the same capture and placement contract instead of reconstructing cursor
/// coordinates in consuming applications.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextMenuAnchor {
    position: Point,
}

/// Captures a secondary-button press over `content` and reports its logical
/// window-space position as a [`ContextMenuAnchor`].
pub struct ContextMenuTrigger<'a, Message> {
    content: Element<'a, Message>,
    on_open: Rc<dyn Fn(ContextMenuAnchor) -> Message + 'a>,
}

impl<'a, Message> ContextMenuTrigger<'a, Message>
where
    Message: 'a,
{
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        on_open: impl Fn(ContextMenuAnchor) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_open: Rc::new(on_open),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        Element::new(self)
    }
}

fn context_menu_anchor(
    event: &Event,
    cursor: mouse::Cursor,
    bounds: Rectangle,
) -> Option<ContextMenuAnchor> {
    if !matches!(
        event,
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
    ) {
        return None;
    }
    cursor
        .position_over(bounds)
        .map(|position| ContextMenuAnchor { position })
}

impl<Message> Widget<Message, Theme, iced::Renderer> for ContextMenuTrigger<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Some(anchor) = context_menu_anchor(event, cursor, layout.bounds()) {
            shell.publish((self.on_open)(anchor));
            shell.capture_event();
            return;
        }
        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
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

/// A searchable context-menu renderer with hover-opened submenus and shared
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

    /// Builds a pointer-triggered context menu at the position captured by
    /// [`ContextMenuTrigger`].
    pub fn at_pointer(
        items: &'a [ContextMenuItem<'a, T>],
        anchor: ContextMenuAnchor,
        viewport: Size,
        on_event: impl Fn(ContextMenuEvent<T>) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self::new(
            items,
            AnchoredMenuPosition::new(anchor.position),
            viewport,
            on_event,
            theme,
        )
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
        let root_panel;
        let root_item_count;
        if self.searchable && !self.query.trim().is_empty() {
            let matches = collect_matches(self.items, self.query);
            root_item_count = matches.len();
            root_panel = self.flat_panel(matches.into_iter().map(|(_, item)| item));
        } else {
            root_item_count = self.items.len();
            root_panel = self.panel(self.items, &[], self.active_path);
        }

        let search_size = ControlSize::Small;
        let menu_size =
            context_menu_panel_size(root_item_count, self.searchable, self.tokens, self.viewport);
        // Fixed list height — avoid Length::Fill collapsing when an ancestor
        // (historically iced::pin) clamps the menu surface near a corner.
        let list_height = context_menu_list_height(menu_size, self.searchable, self.tokens);
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
            scrollable(root_panel)
                .direction(vertical_scrollbar())
                .style(scrollable_style(self.tokens.colors))
                .height(Length::Fixed(list_height)),
        );
        let on_dismiss = (self.on_event)(ContextMenuEvent::Dismiss);
        let on_interaction = (self.on_event)(ContextMenuEvent::Interaction);
        let root = context_menu_surface(content.into(), menu_size, on_interaction, self.tokens);
        let point = self.position.resolve(menu_size, self.viewport);
        Element::new(ContextMenuLayer::new(
            Absolute::new(root, point).into(),
            Rectangle::new(point, menu_size),
            on_dismiss,
        ))
    }

    fn panel(
        &self,
        items: &'a [ContextMenuItem<'a, T>],
        parent_path: &[usize],
        active_path: &[usize],
    ) -> Element<'a, Message> {
        let mut panel = column![]
            .spacing(MENU_ITEM_SPACING)
            .width(Length::Fixed(MENU_PANEL_WIDTH));
        for (index, item) in items.iter().enumerate() {
            let mut path = parent_path.to_vec();
            path.push(index);
            let submenu_active =
                !item.disabled && !item.children.is_empty() && active_path.first() == Some(&index);
            let press_event = if !item.children.is_empty() {
                ContextMenuEvent::OpenSubmenu(path.clone())
            } else {
                ContextMenuEvent::Select(item.value.clone())
            };
            let hover_event = ContextMenuEvent::OpenSubmenu(submenu_hover_path(
                parent_path,
                index,
                !item.disabled && !item.children.is_empty(),
            ));
            let trigger = mouse_area(self.item(item, press_event, submenu_active))
                .on_enter((self.on_event)(hover_event))
                .into();
            let row = if !item.children.is_empty() {
                let surface = self.submenu_surface(
                    &item.children,
                    &path,
                    active_path.get(1..).unwrap_or_default(),
                );
                Element::new(SubmenuAnchor::new(trigger, surface, submenu_active))
            } else {
                trigger
            };
            panel = panel.push(row);
        }
        container(panel).into()
    }

    fn flat_panel(
        &self,
        items: impl IntoIterator<Item = &'a ContextMenuItem<'a, T>>,
    ) -> Element<'a, Message> {
        let mut panel = column![]
            .spacing(MENU_ITEM_SPACING)
            .width(Length::Fixed(MENU_PANEL_WIDTH));
        for item in items {
            panel =
                panel.push(self.item(item, ContextMenuEvent::Select(item.value.clone()), false));
        }
        container(panel).into()
    }

    fn item(
        &self,
        item: &'a ContextMenuItem<'a, T>,
        press_event: ContextMenuEvent<T>,
        active: bool,
    ) -> Element<'a, Message> {
        let pending = self.pending == Some(&item.value);
        let label = if pending {
            item.confirm_label
                .clone()
                .unwrap_or_else(|| item.label.clone())
        } else {
            item.label.clone()
        };
        let mut menu_item = ActionMenuItem::new(label)
            .active(pending || active)
            .danger(item.danger || pending)
            .disabled(item.disabled)
            .on_press((self.on_event)(press_event));
        if let Some(icon) = item.icon {
            menu_item = menu_item.leading(icon);
        }
        if !item.children.is_empty() {
            menu_item = menu_item.hint("›");
        } else if let Some(hint) = &item.hint {
            menu_item = menu_item.hint(hint.clone());
        }
        menu_item.view(self.tokens)
    }

    fn submenu_surface(
        &self,
        items: &'a [ContextMenuItem<'a, T>],
        parent_path: &[usize],
        active_path: &[usize],
    ) -> Element<'a, Message> {
        let size = context_menu_panel_size(items.len(), false, self.tokens, self.viewport);
        let list_height = context_menu_list_height(size, false, self.tokens);
        let content = scrollable(self.panel(items, parent_path, active_path))
            .direction(vertical_scrollbar())
            .style(scrollable_style(self.tokens.colors))
            .height(Length::Fixed(list_height));
        context_menu_surface(
            content.into(),
            size,
            (self.on_event)(ContextMenuEvent::Interaction),
            self.tokens,
        )
    }
}

fn submenu_hover_path(parent_path: &[usize], index: usize, opens_submenu: bool) -> Vec<usize> {
    let mut path = parent_path.to_vec();
    if opens_submenu {
        path.push(index);
    }
    path
}

fn context_menu_surface<'a, Message>(
    content: Element<'a, Message>,
    size: Size,
    on_interaction: Message,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    mouse_area(
        container(content)
            .width(Length::Fixed(size.width))
            .height(Length::Fixed(size.height))
            .padding(MENU_SURFACE_PADDING)
            .style(menu_surface_style(tokens)),
    )
    .on_press(on_interaction)
    .into()
}

/// Captures dismissal before the desktop content beneath the menu sees it.
/// Submenu overlays receive events first; a handled submenu interaction never
/// reaches this layer, while Escape and presses outside the root are consumed.
struct ContextMenuLayer<'a, Message> {
    content: Element<'a, Message>,
    root_bounds: Rectangle,
    on_dismiss: Message,
}

impl<'a, Message> ContextMenuLayer<'a, Message> {
    fn new(content: Element<'a, Message>, root_bounds: Rectangle, on_dismiss: Message) -> Self {
        Self {
            content,
            root_bounds,
            on_dismiss,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextMenuInput {
    Dismiss,
    Capture,
    Forward,
}

fn context_menu_input(
    event: &Event,
    cursor: mouse::Cursor,
    root_bounds: Rectangle,
) -> ContextMenuInput {
    if matches!(
        event,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        })
    ) {
        return ContextMenuInput::Dismiss;
    }
    let pointer_press = match event {
        Event::Mouse(mouse::Event::ButtonPressed(button)) => Some((*button, cursor.position())),
        Event::Touch(touch::Event::FingerPressed { position, .. }) => {
            Some((mouse::Button::Left, Some(*position)))
        }
        _ => None,
    };
    let Some((button, position)) = pointer_press else {
        return ContextMenuInput::Forward;
    };
    if !position.is_some_and(|position| root_bounds.contains(position)) {
        ContextMenuInput::Dismiss
    } else if button != mouse::Button::Left {
        ContextMenuInput::Capture
    } else {
        ContextMenuInput::Forward
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for ContextMenuLayer<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        match context_menu_input(event, cursor, self.root_bounds) {
            ContextMenuInput::Dismiss => {
                shell.publish(self.on_dismiss.clone());
                shell.capture_event();
                return;
            }
            ContextMenuInput::Capture => {
                shell.capture_event();
                return;
            }
            ContextMenuInput::Forward => {}
        }
        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

fn context_menu_panel_size(
    item_count: usize,
    searchable: bool,
    tokens: ThemeTokens,
    viewport: Size,
) -> Size {
    let width = (MENU_SURFACE_PADDING * 2.0 + MENU_PANEL_WIDTH).min(viewport.width.max(0.0));
    let item_height = ControlSize::Small.height_in(tokens.metrics);
    let panel_height =
        item_count as f32 * item_height + item_count.saturating_sub(1) as f32 * MENU_ITEM_SPACING;
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

fn context_menu_list_height(size: Size, searchable: bool, tokens: ThemeTokens) -> f32 {
    let search_height = if searchable {
        ControlSize::Small.height_in(tokens.metrics)
    } else {
        0.0
    };
    let content_spacing = if searchable {
        MENU_CONTENT_SPACING
    } else {
        0.0
    };
    (size.height - MENU_SURFACE_PADDING * 2.0 - search_height - content_spacing).max(0.0)
}

fn resolve_submenu_position(trigger: Rectangle, surface: Size, viewport: Size) -> Point {
    let right = trigger.x + trigger.width + MENU_SURFACE_PADDING;
    let left = trigger.x - MENU_SURFACE_PADDING - surface.width;
    let max_x = (viewport.width - surface.width).max(0.0);
    let x = if right + surface.width <= viewport.width {
        right
    } else if left >= 0.0 {
        left
    } else {
        right.clamp(0.0, max_x)
    };
    let y =
        (trigger.y - MENU_SURFACE_PADDING).clamp(0.0, (viewport.height - surface.height).max(0.0));
    Point::new(x, y)
}

struct SubmenuAnchor<'a, Message> {
    trigger: Element<'a, Message>,
    surface: Element<'a, Message>,
    open: bool,
}

impl<'a, Message> SubmenuAnchor<'a, Message> {
    fn new(trigger: Element<'a, Message>, surface: Element<'a, Message>, open: bool) -> Self {
        Self {
            trigger,
            surface,
            open,
        }
    }
}

#[derive(Debug, Default)]
struct SubmenuAnchorState;

impl<Message> Widget<Message, Theme, iced::Renderer> for SubmenuAnchor<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<SubmenuAnchorState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(SubmenuAnchorState)
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.trigger.as_widget_mut(), self.surface.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        self.trigger.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.trigger
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.trigger.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.trigger.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.trigger.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.trigger
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        let mut children = tree.children.iter_mut();
        let trigger_overlay = self.trigger.as_widget_mut().overlay(
            children.next().expect("submenu trigger state"),
            layout,
            renderer,
            viewport,
            translation,
        );
        let surface_tree = children.next().expect("submenu surface state");
        if !self.open {
            return trigger_overlay;
        }
        let submenu = overlay::Element::new(Box::new(SubmenuOverlay {
            trigger_bounds: layout.bounds() + translation,
            surface: &mut self.surface,
            tree: surface_tree,
        }));
        let overlays: Vec<_> = trigger_overlay.into_iter().chain([submenu]).collect();
        Some(overlay::Group::with_children(overlays).overlay())
    }
}

struct SubmenuOverlay<'a, 'b, Message> {
    trigger_bounds: Rectangle,
    surface: &'b mut Element<'a, Message>,
    tree: &'b mut widget::Tree,
}

impl<Message> overlay::Overlay<Message, Theme, iced::Renderer> for SubmenuOverlay<'_, '_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let surface = self.surface.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let size = surface.size();
        let point = resolve_submenu_position(self.trigger_bounds, size, bounds);
        layout::Node::with_children(size, vec![surface]).move_to(point)
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let non_primary_press = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if *button != mouse::Button::Left
        );
        if non_primary_press && cursor.is_over(layout.bounds()) {
            shell.capture_event();
            return;
        }
        self.surface.as_widget_mut().update(
            self.tree,
            event,
            layout.children().next().expect("submenu surface layout"),
            cursor,
            renderer,
            shell,
            &Rectangle::with_size(Size::INFINITE),
        );
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.surface.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout.children().next().expect("submenu surface layout"),
            cursor,
            &Rectangle::with_size(Size::INFINITE),
        );
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.surface.as_widget_mut().operate(
            self.tree,
            layout.children().next().expect("submenu surface layout"),
            renderer,
            operation,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.surface.as_widget().mouse_interaction(
            self.tree,
            layout.children().next().expect("submenu surface layout"),
            cursor,
            &Rectangle::with_size(Size::INFINITE),
            renderer,
        )
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, iced::Renderer>> {
        self.surface.as_widget_mut().overlay(
            self.tree,
            layout.children().next().expect("submenu surface layout"),
            renderer,
            &Rectangle::with_size(Size::INFINITE),
            Vector::ZERO,
        )
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
        ActionMenuItem, AnchoredMenuPlacement, AnchoredMenuPosition, ContextMenuInput,
        ContextMenuItem, ControlSize, MENU_SURFACE_PADDING, SubmenuAnchor, collect_matches,
        context_menu_anchor, context_menu_input, context_menu_panel_size, resolve_submenu_position,
        submenu_hover_path,
    };
    use crate::theme::ThemeModeExt;
    use iced::advanced::widget::{self, Widget};
    use iced::widget::text;
    use iced::{Event, Point, Rectangle, Size, keyboard, mouse};

    #[test]
    fn anchored_menu_position_stays_inside_the_viewport() {
        let position = AnchoredMenuPosition::new(Point::new(395.0, 295.0))
            .placement(AnchoredMenuPlacement::BottomEnd)
            .resolve(Size::new(180.0, 120.0), Size::new(400.0, 300.0));
        assert_eq!(position, Point::new(215.0, 180.0));
    }

    #[test]
    fn context_menu_anchor_is_the_secondary_press_position_inside_the_trigger() {
        let bounds = Rectangle::new(Point::new(20.0, 30.0), Size::new(120.0, 80.0));
        let secondary = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right));
        let primary = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));

        let anchor = context_menu_anchor(
            &secondary,
            mouse::Cursor::Available(Point::new(72.0, 64.0)),
            bounds,
        );
        assert_eq!(
            anchor.map(|anchor| anchor.position),
            Some(Point::new(72.0, 64.0))
        );
        assert_eq!(
            context_menu_anchor(
                &secondary,
                mouse::Cursor::Available(Point::new(12.0, 18.0)),
                bounds,
            ),
            None
        );
        assert_eq!(
            context_menu_anchor(
                &primary,
                mouse::Cursor::Available(Point::new(72.0, 64.0)),
                bounds,
            ),
            None
        );
    }

    #[test]
    fn pointer_anchor_is_the_menu_origin_when_the_viewport_has_room() {
        let anchor = context_menu_anchor(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            mouse::Cursor::Available(Point::new(72.0, 64.0)),
            Rectangle::new(Point::ORIGIN, Size::new(400.0, 300.0)),
        )
        .expect("secondary press inside trigger");

        assert_eq!(
            AnchoredMenuPosition::new(anchor.position)
                .resolve(Size::new(180.0, 120.0), Size::new(400.0, 300.0)),
            Point::new(72.0, 64.0)
        );
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
        let size = context_menu_panel_size(1, false, tokens, Size::new(800.0, 600.0));
        let item_height = ControlSize::Small.height_in(tokens.metrics);

        assert_eq!(size.width, 200.0);
        assert_eq!(size.height, item_height + 8.0);
    }

    #[test]
    fn context_menu_size_accounts_for_search_without_resizing_for_submenus() {
        let tokens = crate::theme::ThemeMode::Dark.colors().into();
        let size = context_menu_panel_size(3, true, tokens, Size::new(800.0, 600.0));
        let item_height = ControlSize::Small.height_in(tokens.metrics);

        assert_eq!(size.width, 200.0);
        assert_eq!(
            size.height,
            8.0 + item_height + 4.0 + item_height * 3.0 + 2.0
        );
    }

    #[test]
    fn context_menu_size_caps_long_panels_to_the_viewport() {
        let tokens = crate::theme::ThemeMode::Dark.colors().into();
        let viewport = Size::new(320.0, 180.0);
        let size = context_menu_panel_size(100, true, tokens, viewport);

        assert_eq!(size.height, viewport.height);
        let position = AnchoredMenuPosition::new(Point::new(310.0, 170.0))
            .placement(AnchoredMenuPlacement::BottomEnd)
            .resolve(size, viewport);
        assert_eq!(position, Point::new(110.0, 0.0));
    }

    #[test]
    fn submenu_flips_left_and_clamps_vertically_without_moving_its_parent() {
        let trigger = Rectangle::new(Point::new(350.0, 260.0), Size::new(28.0, 28.0));
        let viewport = Size::new(400.0, 300.0);
        let surface = Size::new(200.0, 150.0);

        assert_eq!(
            resolve_submenu_position(trigger, surface, viewport),
            Point::new(trigger.x - MENU_SURFACE_PADDING - surface.width, 150.0)
        );
    }

    #[test]
    fn submenu_prefers_the_right_when_the_window_has_room() {
        let trigger = Rectangle::new(Point::new(40.0, 80.0), Size::new(192.0, 28.0));

        assert_eq!(
            resolve_submenu_position(trigger, Size::new(200.0, 120.0), Size::new(800.0, 600.0),),
            Point::new(
                trigger.x + trigger.width + MENU_SURFACE_PADDING,
                trigger.y - MENU_SURFACE_PADDING,
            )
        );
    }

    #[test]
    fn submenu_hover_opens_branches_and_leaf_hover_returns_to_the_parent_panel() {
        assert_eq!(submenu_hover_path(&[], 2, true), vec![2]);
        assert_eq!(submenu_hover_path(&[2], 1, true), vec![2, 1]);
        assert_eq!(submenu_hover_path(&[2], 3, false), vec![2]);
        assert!(submenu_hover_path(&[], 3, false).is_empty());
    }

    #[test]
    fn submenu_visibility_changes_without_replacing_the_trigger_widget_tree() {
        let mut closed: SubmenuAnchor<'_, ()> =
            SubmenuAnchor::new(text("trigger").into(), text("surface").into(), false);
        let mut tree = widget::Tree::new(&closed as &dyn Widget<(), iced::Theme, iced::Renderer>);
        closed.diff(&mut tree);
        assert_eq!(tree.children.len(), 2);

        let mut open: SubmenuAnchor<'_, ()> =
            SubmenuAnchor::new(text("trigger").into(), text("surface").into(), true);
        open.diff(&mut tree);

        assert_eq!(tree.children.len(), 2);
        assert_eq!(closed.tag(), open.tag());
    }

    #[test]
    fn context_menu_dismissal_consumes_escape_and_outside_presses() {
        let bounds = Rectangle::new(Point::new(40.0, 30.0), Size::new(200.0, 120.0));
        let escape = Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Escape),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        });
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));

        assert_eq!(
            context_menu_input(&escape, mouse::Cursor::Unavailable, bounds),
            ContextMenuInput::Dismiss
        );
        assert_eq!(
            context_menu_input(
                &press,
                mouse::Cursor::Available(Point::new(10.0, 10.0)),
                bounds,
            ),
            ContextMenuInput::Dismiss
        );
        assert_eq!(
            context_menu_input(
                &press,
                mouse::Cursor::Available(Point::new(60.0, 60.0)),
                bounds,
            ),
            ContextMenuInput::Forward
        );
    }
}
