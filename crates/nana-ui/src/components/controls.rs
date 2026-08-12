use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;
use std::rc::Rc;

use iced::advanced::text::highlighter::PlainText;
use iced::advanced::widget::{self as advanced_widget, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, renderer};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, slider, text, text_editor, text_input,
    toggler,
};
use iced::{Alignment, Element, Event, Length, Padding, Pixels, Rectangle, Size, Theme, font};

use crate::components::ControlSize;
use crate::components::tab_drag::{DraggableTabStrip, TabDragSetup};
pub use crate::components::tab_drag::{TabDragGroup, TabDragSurface};
use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{
    SEGMENTED_CONTROL_INSET, checkbox_style, pick_list_menu_style, pick_list_style,
    segmented_button_style, segmented_surface_style, selection_button_style, slider_style,
    text_editor_style, text_input_style, toggler_style,
};

/// A labeled checkbox with native disabled and invalid states.
pub struct Checkbox<'a, Message> {
    checked: bool,
    label: Cow<'a, str>,
    on_toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
    size: ControlSize,
    disabled: bool,
    invalid: bool,
}

impl<'a, Message> Checkbox<'a, Message>
where
    Message: 'a,
{
    pub fn new(checked: bool, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            checked,
            label: label.into(),
            on_toggle: None,
            size: ControlSize::Medium,
            disabled: false,
            invalid: false,
        }
    }

    pub fn on_toggle(mut self, on_toggle: impl Fn(bool) -> Message + 'a) -> Self {
        self.on_toggle = Some(Box::new(on_toggle));
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let control = checkbox(self.checked)
            .label(self.label)
            .size(16)
            .spacing(8)
            .text_size(13)
            .style(checkbox_style(colors, self.invalid));
        let control: Element<'a, Message> = if self.disabled {
            control.into()
        } else if let Some(on_toggle) = self.on_toggle {
            control.on_toggle(on_toggle).into()
        } else {
            control.into()
        };
        container(control)
            .height(Length::Fixed(self.size.height_in(tokens.metrics)))
            .align_y(iced::alignment::Vertical::Center)
            .into()
    }
}

/// A switch with optional supporting hint.
pub struct Switch<'a, Message> {
    toggled: bool,
    label: Cow<'a, str>,
    hint: Option<Cow<'a, str>>,
    on_toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
    size: ControlSize,
    disabled: bool,
    invalid: bool,
}

impl<'a, Message> Switch<'a, Message>
where
    Message: 'a,
{
    pub fn new(toggled: bool, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            toggled,
            label: label.into(),
            hint: None,
            on_toggle: None,
            size: ControlSize::Medium,
            disabled: false,
            invalid: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn on_toggle(mut self, on_toggle: impl Fn(bool) -> Message + 'a) -> Self {
        self.on_toggle = Some(Box::new(on_toggle));
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let has_hint = self.hint.is_some();
        let mut labels = column![
            text(self.label)
                .size(self.size.text_size())
                .font(ui_font(font::Weight::Medium))
                .color(colors.text)
        ]
        .spacing(2)
        .width(Length::Fill);
        if let Some(hint) = self.hint {
            labels = labels.push(text(hint).size(11).color(colors.muted));
        }
        let control = toggler(self.toggled)
            .size(16)
            .style(toggler_style(colors, self.invalid));
        let control: Element<'a, Message> = if self.disabled {
            control.into()
        } else if let Some(on_toggle) = self.on_toggle {
            control.on_toggle(on_toggle).into()
        } else {
            control.into()
        };
        let content = row![labels, control].spacing(10).align_y(Alignment::Center);
        if has_hint {
            content.into()
        } else {
            container(content)
                .height(Length::Fixed(self.size.height_in(tokens.metrics)))
                .align_y(iced::alignment::Vertical::Center)
                .into()
        }
    }
}

/// A single-line text field using NanaUI's shared validation style.
pub struct Input<'a, Message> {
    placeholder: Cow<'a, str>,
    value: Cow<'a, str>,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    size: ControlSize,
    padding: Option<Padding>,
    line_height: Option<f32>,
    disabled: bool,
    invalid: bool,
    secure: bool,
}

impl<'a, Message> Input<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(placeholder: impl Into<Cow<'a, str>>, value: impl Into<Cow<'a, str>>) -> Self {
        Self {
            placeholder: placeholder.into(),
            value: value.into(),
            on_input: None,
            size: ControlSize::Medium,
            padding: None,
            line_height: None,
            disabled: false,
            invalid: false,
            secure: false,
        }
    }

    pub fn on_input(mut self, on_input: impl Fn(String) -> Message + 'a) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = Some(padding.into());
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height.max(1.0));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// Masks the field value while preserving the caller-owned input state.
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let padding = self.padding.unwrap_or_else(|| {
            Padding::from([
                self.size.vertical_padding(tokens.metrics),
                self.size.padding_x(),
            ])
        });
        let line_height = self.line_height.unwrap_or_else(|| self.size.line_height());
        let field = text_input(self.placeholder.as_ref(), self.value.as_ref())
            .secure(self.secure)
            .padding(padding)
            .size(self.size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                line_height,
            )))
            .width(Length::Fill)
            .style(text_input_style(tokens, self.invalid));
        if self.disabled {
            field.into()
        } else if let Some(on_input) = self.on_input {
            field.on_input(on_input).into()
        } else {
            field.into()
        }
    }
}

/// A multi-line text field backed by the caller-owned Iced content model.
pub struct Textarea<'a, Message> {
    content: &'a text_editor::Content,
    placeholder: Cow<'a, str>,
    on_action: Option<Box<dyn Fn(text_editor::Action) -> Message + 'a>>,
    id: Option<advanced_widget::Id>,
    key_binding: Option<TextareaKeyBinding<'a, Message>>,
    disabled: bool,
    invalid: bool,
    height: f32,
}

type TextareaKeyBinding<'a, Message> =
    Box<dyn Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Message>> + 'a>;

impl<'a, Message> Textarea<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: &'a text_editor::Content) -> Self {
        Self {
            content,
            placeholder: Cow::Borrowed(""),
            on_action: None,
            id: None,
            key_binding: None,
            disabled: false,
            invalid: false,
            height: 96.0,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Cow<'a, str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn on_action(mut self, on_action: impl Fn(text_editor::Action) -> Message + 'a) -> Self {
        self.on_action = Some(Box::new(on_action));
        self
    }

    /// Sets a stable widget ID for focus and operation targeting.
    pub fn id(mut self, id: impl Into<advanced_widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets a custom key binding while leaving fallback behavior to the caller.
    ///
    /// Use [`text_editor::Binding::from_key_press`] to preserve Iced's default
    /// editing bindings when the custom binding does not handle a key.
    pub fn key_binding(
        mut self,
        key_binding: impl Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Message>> + 'a,
    ) -> Self {
        self.key_binding = Some(Box::new(key_binding));
        self
    }

    /// Submits on Enter and preserves Shift+Enter as a newline.
    pub fn submit_on_enter(self, message: Message) -> Self {
        self.key_binding(move |key_press| submit_on_enter_binding(key_press, &message))
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(ControlSize::Medium.height());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let mut editor = text_editor(self.content)
            .placeholder(self.placeholder)
            .height(Length::Fixed(self.height))
            .padding(tokens.metrics.field_padding_x)
            .size(13)
            .line_height(iced::widget::text::LineHeight::Relative(1.45))
            .style(text_editor_style(tokens, self.invalid));
        if let Some(id) = self.id {
            editor = editor.id(id);
        }
        if let Some(key_binding) = self.key_binding {
            editor = editor.key_binding(key_binding);
        }
        if self.disabled {
            editor.into()
        } else if let Some(on_action) = self.on_action {
            editor.on_action(on_action).into()
        } else {
            editor.into()
        }
    }
}

fn submit_on_enter_binding<Message: Clone>(
    key_press: text_editor::KeyPress,
    message: &Message,
) -> Option<text_editor::Binding<Message>> {
    let is_focused = matches!(key_press.status, text_editor::Status::Focused { .. });
    let is_enter = matches!(
        key_press.key.as_ref(),
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)
    );
    if is_focused && is_enter && !key_press.modifiers.shift() {
        Some(text_editor::Binding::Custom(message.clone()))
    } else {
        text_editor::Binding::from_key_press(key_press)
    }
}

/// Owned text content that can be shared with a retained hosted UI tree.
///
/// Clones refer to the same content. The hosted runtime and application update
/// this state on the UI thread, so no leaked or self-referential borrow is
/// required to produce an [`Element<'static, _>`](Element).
#[derive(Clone, Default)]
pub struct HostedTextareaState {
    content: Rc<RefCell<text_editor::Content>>,
}

impl HostedTextareaState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: &str) -> Self {
        Self {
            content: Rc::new(RefCell::new(text_editor::Content::with_text(text))),
        }
    }

    pub fn perform(&self, action: text_editor::Action) {
        self.content.borrow_mut().perform(action);
    }

    pub fn set_text(&self, text: &str) {
        *self.content.borrow_mut() = text_editor::Content::with_text(text);
    }

    pub fn clear(&self) {
        self.set_text("");
    }

    pub fn text(&self) -> String {
        self.content.borrow().text()
    }

    pub fn is_empty(&self) -> bool {
        self.content.borrow().is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.content.borrow().line_count()
    }
}

/// A multi-line text field that owns a shared content handle and can therefore
/// be retained by the hosted renderer.
pub struct HostedTextarea<Message> {
    state: HostedTextareaState,
    placeholder: String,
    on_action: Option<Rc<dyn Fn(text_editor::Action) -> Message>>,
    id: Option<advanced_widget::Id>,
    key_binding: Option<HostedTextareaKeyBinding<Message>>,
    disabled: bool,
    invalid: bool,
    height: f32,
}

type HostedTextareaKeyBinding<Message> =
    Rc<dyn Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Message>>>;

impl<Message> HostedTextarea<Message>
where
    Message: Clone + 'static,
{
    pub fn new(state: &HostedTextareaState) -> Self {
        Self {
            state: state.clone(),
            placeholder: String::new(),
            on_action: None,
            id: None,
            key_binding: None,
            disabled: false,
            invalid: false,
            height: 96.0,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn on_action(
        mut self,
        on_action: impl Fn(text_editor::Action) -> Message + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(on_action));
        self
    }

    /// Sets a stable widget ID for focus and operation targeting.
    pub fn id(mut self, id: impl Into<advanced_widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets a custom key binding while leaving fallback behavior to the caller.
    ///
    /// Use [`text_editor::Binding::from_key_press`] to preserve Iced's default
    /// editing bindings when the custom binding does not handle a key.
    pub fn key_binding(
        mut self,
        key_binding: impl Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Message>> + 'static,
    ) -> Self {
        self.key_binding = Some(Rc::new(key_binding));
        self
    }

    /// Submits on Enter and preserves Shift+Enter as a newline.
    pub fn submit_on_enter(self, message: Message) -> Self
    where
        Message: Clone,
    {
        self.key_binding(move |key_press| submit_on_enter_binding(key_press, &message))
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(ControlSize::Medium.height());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'static, Message> {
        let enabled = !self.disabled && self.on_action.is_some();
        Element::new(HostedTextareaWidget {
            state: self.state,
            placeholder: self.placeholder,
            on_action: if enabled { self.on_action } else { None },
            id: self.id,
            key_binding: self.key_binding,
            invalid: self.invalid,
            height: self.height,
            theme: theme.into(),
            status: Cell::new(if enabled {
                text_editor::Status::Active
            } else {
                text_editor::Status::Disabled
            }),
        })
    }
}

struct HostedTextareaWidget<Message> {
    state: HostedTextareaState,
    placeholder: String,
    on_action: Option<Rc<dyn Fn(text_editor::Action) -> Message>>,
    id: Option<advanced_widget::Id>,
    key_binding: Option<HostedTextareaKeyBinding<Message>>,
    invalid: bool,
    height: f32,
    theme: ThemeTokens,
    status: Cell<text_editor::Status>,
}

impl<Message> HostedTextareaWidget<Message>
where
    Message: Clone + 'static,
{
    fn editor<'a>(&'a self, content: &'a text_editor::Content) -> Element<'a, Message> {
        let forced_status = self.status.get();
        let style = text_editor_style(self.theme, self.invalid);
        let mut editor = text_editor(content)
            .placeholder(self.placeholder.as_str())
            .height(Length::Fixed(self.height))
            .padding(self.theme.metrics.field_padding_x)
            .size(13)
            .line_height(iced::widget::text::LineHeight::Relative(1.45))
            .style(move |theme, _status| style(theme, forced_status));
        if let Some(id) = self.id.as_ref() {
            editor = editor.id(id.clone());
        }
        if let Some(key_binding) = self.key_binding.as_ref() {
            let key_binding = Rc::clone(key_binding);
            editor = editor.key_binding(move |key_press| key_binding(key_press));
        }
        if let Some(on_action) = self.on_action.as_ref() {
            let on_action = Rc::clone(on_action);
            editor = editor.on_action(move |action| on_action(action));
        }
        editor.into()
    }

    fn current_status(
        &self,
        tree: &advanced_widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) -> text_editor::Status {
        if self.on_action.is_none() {
            return text_editor::Status::Disabled;
        }
        let state = tree.state.downcast_ref::<text_editor::State<PlainText>>();
        if state.is_focused() {
            text_editor::Status::Focused {
                is_hovered: cursor.is_over(layout.bounds()),
            }
        } else if cursor.is_over(layout.bounds()) {
            text_editor::Status::Hovered
        } else {
            text_editor::Status::Active
        }
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for HostedTextareaWidget<Message>
where
    Message: Clone + 'static,
{
    fn tag(&self) -> advanced_widget::tree::Tag {
        let content = self.state.content.borrow();
        self.editor(&content).as_widget().tag()
    }

    fn state(&self) -> advanced_widget::tree::State {
        let content = self.state.content.borrow();
        self.editor(&content).as_widget().state()
    }

    fn diff(&mut self, tree: &mut advanced_widget::Tree) {
        let content = self.state.content.borrow();
        self.editor(&content).as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        let content = self.state.content.borrow();
        self.editor(&content).as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut advanced_widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let content = self.state.content.borrow();
        self.editor(&content)
            .as_widget_mut()
            .layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut advanced_widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        {
            let content = self.state.content.borrow();
            self.editor(&content)
                .as_widget_mut()
                .update(tree, event, layout, cursor, renderer, shell, viewport);
        }
        let status = self.current_status(tree, layout, cursor);
        if self.status.replace(status) != status {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &advanced_widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let content = self.state.content.borrow();
        self.editor(&content)
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(
        &mut self,
        tree: &mut advanced_widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn advanced_widget::Operation,
    ) {
        let content = self.state.content.borrow();
        self.editor(&content)
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        tree: &advanced_widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let content = self.state.content.borrow();
        self.editor(&content)
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
}

/// A numeric range control with an optional unit readout.
pub struct RangeField<'a, Message> {
    range: RangeInclusive<f32>,
    value: f32,
    on_change: Box<dyn Fn(f32) -> Message + 'a>,
    label: Option<Cow<'a, str>>,
    unit: Option<Cow<'a, str>>,
    size: ControlSize,
}

impl<'a, Message> RangeField<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        range: RangeInclusive<f32>,
        value: f32,
        on_change: impl Fn(f32) -> Message + 'a,
    ) -> Self {
        Self {
            range,
            value,
            on_change: Box::new(on_change),
            label: None,
            unit: None,
            size: ControlSize::Medium,
        }
    }

    pub fn label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn unit(mut self, unit: impl Into<Cow<'a, str>>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut content = row![].spacing(8).align_y(Alignment::Center);
        if let Some(label) = self.label {
            content = content.push(text(label).size(11).color(colors.text));
        }
        content = content.push(
            slider(self.range, self.value, self.on_change)
                .height(16)
                .style(slider_style(colors))
                .width(Length::Fill),
        );
        if let Some(unit) = self.unit {
            content = content.push(
                text(format!("{}{}", format_number(self.value), unit))
                    .size(10)
                    .color(colors.accent),
            );
        }
        container(content)
            .height(Length::Fixed(self.size.height_in(tokens.metrics)))
            .align_y(iced::alignment::Vertical::Center)
            .into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOption<'a, T> {
    pub value: T,
    pub label: Cow<'a, str>,
    pub icon: Option<Icon>,
    pub disabled: bool,
    pub draggable: bool,
}

impl<'a, T> SelectionOption<'a, T> {
    pub fn new(value: T, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            value,
            label: label.into(),
            icon: None,
            disabled: false,
            draggable: true,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Controls whether this option can participate in a reorderable tab drag.
    /// It remains selectable when dragging is disabled.
    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self
    }
}

/// A compact mutually-exclusive selection control.
pub struct SegmentedControl<'a, T, Message> {
    value: T,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    size: ControlSize,
}

impl<'a, T, Message> SegmentedControl<'a, T, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        value: T,
        options: impl IntoIterator<Item = SelectionOption<'a, T>>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            value,
            options: options.into_iter().collect(),
            on_select: Box::new(on_select),
            size: ControlSize::Medium,
        }
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        selection_view(
            self.value,
            self.options,
            self.on_select,
            self.size,
            theme.into(),
            true,
            false,
        )
    }
}

/// A horizontal tab list sharing selection behavior but not the segmented border.
type TabReorderHandler<'a, T, Message> = Box<dyn Fn(T, Option<T>) -> Message + 'a>;
type TabTransferHandler<'a, T, Message> = Box<dyn Fn(String, T, String, Option<T>) -> Message + 'a>;
type TabDragGroupConfig<'a, T, Message> = (
    TabDragGroup<T>,
    TabDragSurface,
    String,
    TabTransferHandler<'a, T, Message>,
);

pub struct Tabs<'a, T, Message> {
    value: T,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    on_reorder: Option<TabReorderHandler<'a, T, Message>>,
    drag_group: Option<TabDragGroupConfig<'a, T, Message>>,
    accepts_external_drop: bool,
    size: ControlSize,
    fill: bool,
}

/// A native single-value select. Disabled options stay visible as the selected
/// value but are omitted from the selectable popup because Iced's native
/// pick-list has no per-option disabled event contract.
pub struct Select<'a, T, Message> {
    value: Option<T>,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    placeholder: Option<Cow<'a, str>>,
    size: ControlSize,
    disabled: bool,
    loading: bool,
    invalid: bool,
}

impl<'a, T, Message> Select<'a, T, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        value: Option<T>,
        options: impl IntoIterator<Item = SelectionOption<'a, T>>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            value,
            options: options.into_iter().collect(),
            on_select: Box::new(on_select),
            placeholder: None,
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Cow<'a, str>>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let selected = self
            .value
            .as_ref()
            .and_then(|value| self.options.iter().find(|option| &option.value == value))
            .cloned();
        let enabled_options: Vec<_> = self
            .options
            .into_iter()
            .filter(|option| !option.disabled)
            .collect();
        let mut control = pick_list(selected, enabled_options, |option| option.label.to_string())
            .width(Length::Fill)
            .padding([
                self.size.vertical_padding(tokens.metrics),
                self.size.padding_x(),
            ])
            .text_size(self.size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                self.size.line_height(),
            )))
            .style(pick_list_style(tokens, self.invalid))
            .menu_style(pick_list_menu_style(tokens));
        if let Some(placeholder) = self.placeholder {
            control = control.placeholder(placeholder);
        }
        if !self.disabled && !self.loading {
            let on_select = self.on_select;
            control = control.on_select(move |option| on_select(option.value));
        }
        control.into()
    }
}

impl<'a, T, Message> Tabs<'a, T, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        value: T,
        options: impl IntoIterator<Item = SelectionOption<'a, T>>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            value,
            options: options.into_iter().collect(),
            on_select: Box::new(on_select),
            on_reorder: None,
            drag_group: None,
            accepts_external_drop: true,
            size: ControlSize::Small,
            fill: false,
        }
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Distributes all tabs evenly across the available horizontal space.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Enables pointer and touch reordering within this tab strip.
    ///
    /// `before` identifies the tab that will follow the moved value. `None`
    /// appends it to the end. Selection and ordering remain application-owned.
    pub fn on_reorder(mut self, on_reorder: impl Fn(T, Option<T>) -> Message + 'a) -> Self {
        self.on_reorder = Some(Box::new(on_reorder));
        self
    }

    /// Joins this strip to a shared pointer/touch drag group.
    ///
    /// The transfer handler runs only when a tab is released over another
    /// registered strip. `source_strip` and `target_strip` are application-owned
    /// stable IDs; `before` follows the same ordering contract as
    /// [`Tabs::on_reorder`].
    pub fn drag_group(
        self,
        group: TabDragGroup<T>,
        strip_id: impl Into<String>,
        on_transfer: impl Fn(String, T, String, Option<T>) -> Message + 'a,
    ) -> Self {
        self.drag_group_on_surface(group, TabDragSurface::new("default"), strip_id, on_transfer)
    }

    /// Joins a drag group using window-local coordinates mapped to physical
    /// screen space by `surface`.
    pub fn drag_group_on_surface(
        mut self,
        group: TabDragGroup<T>,
        surface: TabDragSurface,
        strip_id: impl Into<String>,
        on_transfer: impl Fn(String, T, String, Option<T>) -> Message + 'a,
    ) -> Self {
        self.drag_group = Some((group, surface, strip_id.into(), Box::new(on_transfer)));
        self
    }

    /// Controls whether this strip can receive tabs from another strip.
    /// The strip remains a valid drag source when external drops are disabled.
    pub fn accepts_external_drop(mut self, accepts: bool) -> Self {
        self.accepts_external_drop = accepts;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let values = self
            .options
            .iter()
            .map(|option| option.value.clone())
            .collect::<Vec<_>>();
        let disabled = self
            .options
            .iter()
            .map(|option| option.disabled || !option.draggable)
            .collect::<Vec<_>>();
        let on_select: Rc<dyn Fn(T) -> Message + 'a> = Rc::from(self.on_select);
        let content_on_select = Rc::clone(&on_select);
        let content = selection_view(
            self.value,
            self.options,
            Box::new(move |value| content_on_select(value)),
            self.size,
            tokens,
            false,
            self.fill,
        );
        if self.on_reorder.is_none() && self.drag_group.is_none() {
            return content;
        }
        DraggableTabStrip::new(
            content,
            values,
            disabled,
            on_select,
            self.on_reorder.map(Rc::from),
            self.drag_group
                .map(|(group, surface, strip_id, on_transfer)| {
                    TabDragSetup::new(
                        group,
                        surface,
                        strip_id,
                        Rc::from(on_transfer),
                        self.accepts_external_drop,
                    )
                }),
            tokens.colors.accent,
        )
        .into()
    }
}

fn selection_view<'a, T, Message>(
    value: T,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    size: ControlSize,
    tokens: ThemeTokens,
    segmented: bool,
    fill: bool,
) -> Element<'a, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    let colors = tokens.colors;
    let height = size.height_in(tokens.metrics);
    let option_height = if segmented {
        (height - SEGMENTED_CONTROL_INSET * 2.0).max(0.0)
    } else {
        height
    };
    let mut options_row = row![]
        .spacing(if segmented { 2 } else { 4 })
        .width(if fill { Length::Fill } else { Length::Shrink });
    for option in options {
        let selected = option.value == value;
        let mut content = row![].spacing(5).align_y(Alignment::Center);
        if let Some(option_icon) = option.icon {
            content = content.push(icon(
                option_icon,
                size.icon_size(),
                if selected { colors.text } else { colors.muted },
            ));
        }
        content = content.push(
            text(option.label)
                .size(size.text_size())
                .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                    size.line_height(),
                )))
                .font(ui_font(font::Weight::Medium)),
        );
        let message = (!option.disabled).then(|| on_select(option.value));
        let option_button = button(content)
            .width(if fill {
                Length::FillPortion(1)
            } else {
                Length::Shrink
            })
            .height(Length::Fixed(option_height))
            .padding([0.0, size.padding_x() + 2.0])
            .on_press_maybe(message);
        options_row = options_row.push(if segmented {
            option_button.style(segmented_button_style(tokens, selected))
        } else {
            option_button.style(selection_button_style(tokens, selected))
        });
    }
    if segmented {
        container(options_row)
            .height(Length::Fixed(height))
            .padding(SEGMENTED_CONTROL_INSET)
            .style(segmented_surface_style(tokens))
            .into()
    } else {
        options_row.into()
    }
}

fn format_number(value: f32) -> String {
    if value.fract().abs() < f32::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Checkbox, ControlSize, HostedTextarea, HostedTextareaState, Input, RangeField,
        SegmentedControl, Select, SelectionOption, Switch, Tabs, format_number,
        submit_on_enter_binding,
    };
    use crate::theme::ThemeMode;
    use iced::Element;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::{Key, Modifiers};
    use iced::widget::text_editor;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestMessage {
        Edit,
        Submit,
    }

    fn enter_key_press(modifiers: Modifiers, status: text_editor::Status) -> text_editor::KeyPress {
        let key = Key::Named(Named::Enter);
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(Code::Enter),
            modifiers,
            text: None,
            status,
        }
    }

    #[test]
    fn range_readout_keeps_integers_compact_without_losing_fractional_values() {
        assert_eq!(format_number(8.0), "8");
        assert_eq!(format_number(8.25), "8.2");
    }

    #[test]
    fn single_line_controls_use_the_contextual_default_tiers() {
        assert_eq!(Checkbox::<()>::new(false, "选项").size, ControlSize::Medium);
        assert_eq!(Switch::<()>::new(false, "开关").size, ControlSize::Medium);
        assert_eq!(Input::<()>::new("", "").size, ControlSize::Medium);
        assert_eq!(
            RangeField::new(0.0..=1.0, 0.5, |_| ()).size,
            ControlSize::Medium
        );
        assert_eq!(
            SegmentedControl::new(false, [SelectionOption::new(false, "关")], |_| ()).size,
            ControlSize::Medium
        );
        assert_eq!(
            Select::new(Some(false), [SelectionOption::new(false, "关")], |_| ()).size,
            ControlSize::Medium
        );
        assert_eq!(
            Tabs::new(false, [SelectionOption::new(false, "标签")], |_| ()).size,
            ControlSize::Small
        );
    }

    #[test]
    fn submit_binding_keeps_shift_enter_as_the_default_newline() {
        let focused = text_editor::Status::Focused { is_hovered: false };

        assert_eq!(
            submit_on_enter_binding(
                enter_key_press(Modifiers::NONE, focused),
                &TestMessage::Submit,
            ),
            Some(text_editor::Binding::Custom(TestMessage::Submit))
        );
        assert_eq!(
            submit_on_enter_binding(
                enter_key_press(Modifiers::SHIFT, focused),
                &TestMessage::Submit,
            ),
            Some(text_editor::Binding::Enter)
        );
        assert_eq!(
            submit_on_enter_binding(
                enter_key_press(Modifiers::NONE, text_editor::Status::Active),
                &TestMessage::Submit,
            ),
            None
        );
    }

    #[test]
    fn hosted_textarea_state_is_shared_without_leaking_content() {
        let state = HostedTextareaState::with_text("draft");
        let shared = state.clone();

        shared.perform(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        assert_eq!(state.text(), "!draft");

        state.clear();
        assert!(shared.is_empty());
        assert_eq!(shared.line_count(), 1);
    }

    #[test]
    fn hosted_textarea_builds_a_static_element_with_a_stable_id() {
        fn assert_static(_: Element<'static, TestMessage>) {}

        let state = HostedTextareaState::new();
        assert_static(
            HostedTextarea::new(&state)
                .id("hosted-textarea-test")
                .on_action(|_| TestMessage::Edit)
                .submit_on_enter(TestMessage::Submit)
                .view(ThemeMode::Dark.tokens()),
        );
    }
}
