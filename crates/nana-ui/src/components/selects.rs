use std::borrow::Cow;
use std::fmt;
use std::rc::Rc;

use iced::widget::{combo_box, pick_list, text_input};
use iced::{Element, Length, Pixels};

use super::ControlSize;
use crate::theme::ThemeTokens;
use crate::widgets::{pick_list_menu_style, pick_list_style, text_input_style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownOption<'a, T> {
    pub value: T,
    pub label: Cow<'a, str>,
    pub hint: Option<Cow<'a, str>>,
    pub disabled: bool,
}

impl<'a, T> DropdownOption<'a, T> {
    pub fn new(value: T, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            value,
            label: label.into(),
            hint: None,
            disabled: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn menu_label(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{}  ·  {hint}", self.label),
            None => self.label.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropdownSelection<T> {
    Single(Option<T>),
    Multiple(Vec<T>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropdownEvent<T> {
    Select(T),
    Toggle(T),
    Opened,
    Closed,
}

/// A native anchored value menu supporting single and multiple selection.
///
/// The single-select path follows normal pick-list behavior. The multiple
/// path commits one toggle per menu invocation and summarizes the selected
/// labels in the trigger without allocating per-frame lookup maps.
pub struct Dropdown<'a, T, Message> {
    selection: DropdownSelection<T>,
    options: Vec<DropdownOption<'a, T>>,
    on_event: Rc<dyn Fn(DropdownEvent<T>) -> Message + 'a>,
    placeholder: Cow<'a, str>,
    display_label: Option<Cow<'a, str>>,
    size: ControlSize,
    width: Length,
    disabled: bool,
    loading: bool,
    invalid: bool,
}

impl<'a, T, Message> Dropdown<'a, T, Message>
where
    T: Clone + PartialEq + 'a,
    Message: Clone + 'a,
{
    pub fn single(
        value: Option<T>,
        options: impl IntoIterator<Item = DropdownOption<'a, T>>,
        on_event: impl Fn(DropdownEvent<T>) -> Message + 'a,
    ) -> Self {
        Self::new(DropdownSelection::Single(value), options, on_event)
    }

    pub fn multiple(
        values: impl IntoIterator<Item = T>,
        options: impl IntoIterator<Item = DropdownOption<'a, T>>,
        on_event: impl Fn(DropdownEvent<T>) -> Message + 'a,
    ) -> Self {
        Self::new(
            DropdownSelection::Multiple(values.into_iter().collect()),
            options,
            on_event,
        )
    }

    fn new(
        selection: DropdownSelection<T>,
        options: impl IntoIterator<Item = DropdownOption<'a, T>>,
        on_event: impl Fn(DropdownEvent<T>) -> Message + 'a,
    ) -> Self {
        Self {
            selection,
            options: options.into_iter().collect(),
            on_event: Rc::new(on_event),
            placeholder: Cow::Borrowed("-"),
            display_label: None,
            size: ControlSize::Medium,
            width: Length::Shrink,
            disabled: false,
            loading: false,
            invalid: false,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Cow<'a, str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn display_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.display_label = Some(label.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
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
        let selected = self.selected_option();
        let enabled_options: Vec<_> = self
            .options
            .into_iter()
            .filter(|option| !option.disabled)
            .collect();
        let on_event = Rc::clone(&self.on_event);
        let multiple = matches!(self.selection, DropdownSelection::Multiple(_));
        let mut control = pick_list(selected, enabled_options, DropdownOption::menu_label)
            .width(self.width)
            .padding([
                self.size.vertical_padding(tokens.metrics),
                self.size.padding_x(),
            ])
            .text_size(self.size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                self.size.line_height(),
            )))
            .placeholder(self.placeholder)
            .style(pick_list_style(tokens, self.invalid))
            .menu_style(pick_list_menu_style(tokens))
            .on_open((self.on_event)(DropdownEvent::Opened))
            .on_close((self.on_event)(DropdownEvent::Closed));
        if !self.disabled && !self.loading {
            control = control.on_select(move |option| {
                on_event(if multiple {
                    DropdownEvent::Toggle(option.value)
                } else {
                    DropdownEvent::Select(option.value)
                })
            });
        }
        control.into()
    }

    fn selected_option(&self) -> Option<DropdownOption<'a, T>> {
        if let Some(label) = &self.display_label {
            return self.synthetic_selected(label.to_string());
        }
        match &self.selection {
            DropdownSelection::Single(value) => value.as_ref().and_then(|value| {
                self.options
                    .iter()
                    .find(|option| &option.value == value)
                    .cloned()
            }),
            DropdownSelection::Multiple(values) => {
                let label = multiple_label(values, &self.options, &self.placeholder);
                (!values.is_empty())
                    .then(|| self.synthetic_selected(label))
                    .flatten()
            }
        }
    }

    fn synthetic_selected(&self, label: String) -> Option<DropdownOption<'a, T>> {
        let fallback = match &self.selection {
            DropdownSelection::Single(Some(value)) => Some(value),
            DropdownSelection::Multiple(values) => values.first(),
            DropdownSelection::Single(None) => None,
        }
        .or_else(|| self.options.first().map(|option| &option.value))?;
        Some(DropdownOption {
            value: fallback.clone(),
            label: Cow::Owned(label),
            hint: None,
            disabled: false,
        })
    }
}

fn multiple_label<T: PartialEq>(
    values: &[T],
    options: &[DropdownOption<'_, T>],
    placeholder: &str,
) -> String {
    let mut labels = options
        .iter()
        .filter(|option| values.contains(&option.value))
        .map(|option| option.label.as_ref());
    let first = labels.next();
    let second = labels.next();
    let remaining = labels.count();
    match (first, second, remaining) {
        (None, _, _) => placeholder.to_owned(),
        (Some(first), None, _) => first.to_owned(),
        (Some(first), Some(second), 0) => format!("{first}, {second}"),
        (Some(first), Some(second), count) => format!("{first}, {second} +{count}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDropdownOption<T> {
    pub value: T,
    pub label: String,
    pub hint: Option<String>,
}

impl<T> SearchDropdownOption<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl<T> fmt::Display for SearchDropdownOption<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)?;
        if let Some(hint) = &self.hint {
            write!(formatter, "  ·  {hint}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SearchDropdownState<T> {
    inner: combo_box::State<SearchDropdownOption<T>>,
}

impl<T> SearchDropdownState<T>
where
    T: Clone + 'static,
{
    pub fn new(options: impl IntoIterator<Item = SearchDropdownOption<T>>) -> Self {
        Self {
            inner: combo_box::State::new(options.into_iter().collect()),
        }
    }

    pub fn options(&self) -> &[SearchDropdownOption<T>] {
        self.inner.options()
    }
}

/// A searchable native dropdown backed by Iced's retained filtering state.
pub struct SearchDropdown<'a, T, Message> {
    state: &'a SearchDropdownState<T>,
    selected: Option<&'a T>,
    on_select: Box<dyn Fn(T) -> Message + 'a>,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    on_open: Option<Message>,
    on_close: Option<Message>,
    placeholder: &'a str,
    size: ControlSize,
    disabled: bool,
    loading: bool,
    invalid: bool,
}

impl<'a, T, Message> SearchDropdown<'a, T, Message>
where
    T: Clone + PartialEq + 'a + 'static,
    Message: Clone + 'a,
{
    pub fn new(
        state: &'a SearchDropdownState<T>,
        selected: Option<&'a T>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            state,
            selected,
            on_select: Box::new(on_select),
            on_input: None,
            on_open: None,
            on_close: None,
            placeholder: "",
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
        }
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn on_input(mut self, on_input: impl Fn(String) -> Message + 'a) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    pub fn on_open(mut self, message: Message) -> Self {
        self.on_open = Some(message);
        self
    }

    pub fn on_close(mut self, message: Message) -> Self {
        self.on_close = Some(message);
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
        let selected = self.selected.and_then(|value| {
            self.state
                .options()
                .iter()
                .find(|option| &option.value == value)
        });
        if self.disabled || self.loading {
            return text_input(
                self.placeholder,
                selected.map_or("", |option| option.label.as_str()),
            )
            .padding([
                self.size.vertical_padding(tokens.metrics),
                self.size.padding_x(),
            ])
            .size(self.size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                self.size.line_height(),
            )))
            .width(Length::Fill)
            .style(text_input_style(tokens, self.invalid))
            .into();
        }

        let on_select = self.on_select;
        let mut control = combo_box(
            &self.state.inner,
            self.placeholder,
            selected,
            move |option| on_select(option.value),
        )
        .width(Length::Fill)
        .padding([
            self.size.vertical_padding(tokens.metrics),
            self.size.padding_x(),
        ])
        .size(self.size.text_size())
        .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
            self.size.line_height(),
        )))
        .input_style(text_input_style(tokens, self.invalid))
        .menu_style(pick_list_menu_style(tokens));
        if let Some(on_input) = self.on_input {
            control = control.on_input(on_input);
        }
        if let Some(message) = self.on_open {
            control = control.on_open(message);
        }
        if let Some(message) = self.on_close {
            control = control.on_close(message);
        }
        control.into()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ControlSize, Dropdown, DropdownOption, SearchDropdown, SearchDropdownOption,
        SearchDropdownState, multiple_label,
    };

    #[test]
    fn multiple_selection_summary_keeps_two_labels_and_a_remaining_count() {
        let options = [
            DropdownOption::new(1, "一"),
            DropdownOption::new(2, "二"),
            DropdownOption::new(3, "三"),
            DropdownOption::new(4, "四"),
        ];
        assert_eq!(multiple_label(&[1, 2, 4], &options, "-"), "一, 二 +1");
        assert_eq!(multiple_label::<i32>(&[], &options, "未选择"), "未选择");
    }

    #[test]
    fn dropdown_families_default_to_the_medium_form_tier() {
        let dropdown = Dropdown::single(None::<u8>, [DropdownOption::new(1, "一")], |_| ());
        assert_eq!(dropdown.size, ControlSize::Medium);

        let state = SearchDropdownState::new([SearchDropdownOption::new(1, "一")]);
        let search = SearchDropdown::new(&state, None, |_| ());
        assert_eq!(search.size, ControlSize::Medium);
    }
}
