//! Element helpers for the controls the prototype covers, with typed
//! bindable fields.

use std::sync::Arc;

use super::node::{El, IntoView, widget};
use super::prop::{FieldWrite, IntoProp};
use super::reactive::Signal;
use crate::{
    Activate, Button, Checkbox, NodeStyle, RangeField, RangeInput, Stack, Text, TextChanged,
    TextInput, ToggleChanged,
};

/// Components whose [`NodeStyle`] the view layer may write (visibility).
pub trait StyledComponent {
    #[doc(hidden)]
    fn node_style(&self) -> &NodeStyle;
    #[doc(hidden)]
    fn node_style_mut(&mut self) -> &mut NodeStyle;
}

macro_rules! styled {
    ($($component:ty),*) => {$(
        impl StyledComponent for $component {
            fn node_style(&self) -> &NodeStyle {
                &self.style
            }

            fn node_style_mut(&mut self) -> &mut NodeStyle {
                &mut self.style
            }
        }
    )*};
}

styled!(Text, Button, TextInput, RangeField, Checkbox);

impl StyledComponent for Stack {
    fn node_style(&self) -> &NodeStyle {
        self.style_ref()
    }

    fn node_style_mut(&mut self) -> &mut NodeStyle {
        self.style_mut()
    }
}

#[doc(hidden)]
pub struct Visible;

impl<C: StyledComponent> FieldWrite<C, bool> for Visible {
    const FIELD: &'static str = "style.layout.hidden";

    fn write(target: &mut C, visible: bool) {
        let style = target.node_style_mut();
        if style.layout.hidden == visible {
            Arc::make_mut(&mut style.layout).hidden = !visible;
        }
    }

    fn differs(target: &C, visible: &bool) -> bool {
        target.node_style().layout.hidden == *visible
    }
}

impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
    /// Keep the node but take it out of layout, paint and hit testing
    /// (`v-show`). Use [`super::when`] to drop the subtree instead (`v-if`).
    #[track_caller]
    pub fn visible(self, visible: impl IntoProp<bool>) -> Self {
        self.prop::<bool, Visible>(visible)
    }
}

/// Generates a `FieldWrite` per field and the matching `El` setter. A field
/// compares against `target.<field>` unless it names its own `differs`.
macro_rules! props {
    ($component:ident { $($field:ident: $ty:ty => $writer:ident |$target:ident, $value:ident| $body:expr $(, differs |$dt:ident, $dv:ident| $differs:expr)?);* $(;)? }) => {
        $(
            #[doc(hidden)]
            pub struct $writer;

            impl FieldWrite<$component, $ty> for $writer {
                const FIELD: &'static str = concat!(stringify!($component), ".", stringify!($field));

                fn write($target: &mut $component, $value: $ty) {
                    $body
                }

                fn differs(target: &$component, value: &$ty) -> bool {
                    props!(@differs target, value, $field $(, |$dt, $dv| $differs)?)
                }
            }
        )*

        impl<K> El<$component, K> {
            $(
                #[track_caller]
                pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                    self.prop::<$ty, $writer>(value)
                }
            )*
        }
    };
    (@differs $target:ident, $value:ident, $field:ident) => {
        $target.$field != *$value
    };
    (@differs $target:ident, $value:ident, $field:ident, |$dt:ident, $dv:ident| $differs:expr) => {{
        let ($dt, $dv) = ($target, $value);
        $differs
    }};
}

props!(Text {
    value: String => TextValue |target, value| target.value = value;
});

props!(Button {
    label: String => ButtonLabel |target, value| target.label = value;
    disabled: bool => ButtonDisabled |target, value| target.disabled = value;
    loading: bool => ButtonLoading |target, value| target.loading = value;
});

props!(RangeField {
    value: f64 => RangeValue |target, value| target.value = value;
    disabled: bool => RangeDisabled |target, value| target.disabled = value;
});

props!(TextInput {
    value: String => TextInputValue |target, value| {
        // Only a different value replaces the text, so echoing an edit back
        // through its signal leaves the caret and selection alone.
        if target.state.value != value {
            target.state.replace_value(value);
        }
    }, differs |target, value| target.state.value != *value;
    placeholder: Arc<str> => TextInputPlaceholder |target, value| target.placeholder = value;
    disabled: bool => TextInputDisabled |target, value| target.disabled = value;
});

props!(Checkbox {
    label: String => CheckboxLabel |target, value| target.label = value;
    checked: bool => CheckboxChecked |target, value| target.checked = value;
    disabled: bool => CheckboxDisabled |target, value| target.disabled = value;
});

/// Vertical stack.
#[track_caller]
pub fn column<K: IntoView>(gap: f32, children: K) -> El<Stack, K> {
    widget(Stack::column(gap)).children(children)
}

/// Horizontal stack.
#[track_caller]
pub fn row<K: IntoView>(gap: f32, children: K) -> El<Stack, K> {
    widget(Stack::row(gap)).children(children)
}

/// Text. For interpolation see [`crate::text!`].
#[track_caller]
pub fn text(value: impl IntoProp<String>) -> El<Text> {
    widget(Text::new("")).value(value)
}

#[track_caller]
pub fn button(label: impl IntoProp<String>) -> El<Button> {
    widget(Button::new("")).label(label)
}

/// A range slider between `minimum` and `maximum`.
#[track_caller]
pub fn slider(minimum: f64, maximum: f64, step: f64) -> El<RangeField> {
    widget(RangeField::new(minimum, minimum, maximum, step))
}

#[track_caller]
pub fn text_input() -> El<TextInput> {
    widget(TextInput::new(""))
}

#[track_caller]
pub fn checkbox(label: impl IntoProp<String>) -> El<Checkbox> {
    widget(Checkbox::new("", false)).label(label)
}

impl<K> El<Button, K> {
    /// `@click`.
    pub fn on_activate(self, mut f: impl FnMut() + Send + 'static) -> Self {
        self.on(move |_: &Activate| f())
    }
}

impl<K> El<RangeField, K> {
    /// `v-model`: the slider shows `value` and every step it takes, drag
    /// included, writes it back.
    #[track_caller]
    pub fn model(self, value: Signal<f64>) -> Self {
        self.value(value)
            .on(move |event: &RangeInput| value.set(event.value))
    }
}

impl<K> El<TextInput, K> {
    /// `v-model` over the field's text.
    #[track_caller]
    pub fn model(self, value: Signal<String>) -> Self {
        self.value(value)
            .on(move |event: &TextChanged| value.set(event.value.to_string()))
    }
}

impl<K> El<Checkbox, K> {
    /// `v-model` over the checked state.
    #[track_caller]
    pub fn model(self, checked: Signal<bool>) -> Self {
        self.checked(checked)
            .on(move |event: &ToggleChanged| checked.set(event.checked))
    }
}
