use std::borrow::Cow;
use std::ops::RangeInclusive;

use iced::widget::{
    button, checkbox, column, container, pick_list, row, slider, text, text_editor, text_input,
    toggler,
};
use iced::{Alignment, Element, Length, Pixels, font};

use crate::components::ControlSize;
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
    placeholder: &'a str,
    value: &'a str,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    size: ControlSize,
    disabled: bool,
    invalid: bool,
}

impl<'a, Message> Input<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(placeholder: &'a str, value: &'a str) -> Self {
        Self {
            placeholder,
            value,
            on_input: None,
            size: ControlSize::Medium,
            disabled: false,
            invalid: false,
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
        let field = text_input(self.placeholder, self.value)
            .padding([
                self.size.vertical_padding(tokens.metrics),
                self.size.padding_x(),
            ])
            .size(self.size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                self.size.line_height(),
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
    disabled: bool,
    invalid: bool,
    height: f32,
}

impl<'a, Message> Textarea<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: &'a text_editor::Content) -> Self {
        Self {
            content,
            placeholder: Cow::Borrowed(""),
            on_action: None,
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
        let editor = text_editor(self.content)
            .placeholder(self.placeholder)
            .height(Length::Fixed(self.height))
            .padding(tokens.metrics.field_padding_x)
            .size(13)
            .line_height(iced::widget::text::LineHeight::Relative(1.45))
            .style(text_editor_style(tokens, self.invalid));
        if self.disabled {
            editor.into()
        } else if let Some(on_action) = self.on_action {
            editor.on_action(on_action).into()
        } else {
            editor.into()
        }
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
}

impl<'a, T> SelectionOption<'a, T> {
    pub fn new(value: T, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            value,
            label: label.into(),
            icon: None,
            disabled: false,
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
        )
    }
}

/// A horizontal tab list sharing selection behavior but not the segmented border.
pub struct Tabs<'a, T, Message> {
    value: T,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    size: ControlSize,
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
            size: ControlSize::Small,
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
            false,
        )
    }
}

fn selection_view<'a, T, Message>(
    value: T,
    options: Vec<SelectionOption<'a, T>>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    size: ControlSize,
    tokens: ThemeTokens,
    segmented: bool,
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
    let mut options_row = row![].spacing(if segmented { 2 } else { 4 });
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
        content = content.push(text(option.label).size(size.text_size()));
        let message = (!option.disabled).then(|| on_select(option.value));
        let option_button = button(content)
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
        Checkbox, ControlSize, Input, RangeField, SegmentedControl, Select, SelectionOption,
        Switch, Tabs, format_number,
    };

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
}
