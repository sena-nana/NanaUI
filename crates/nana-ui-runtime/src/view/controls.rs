//! Element functions of the built-in controls, and their typed bindable
//! fields, `model` and event methods expanded from the control table in
//! `nana-ui-view-schema`.

use std::any::Any;
use std::sync::Arc;

use super::node::{El, IntoView, widget};
use super::prop::{FieldWrite, IntoProp};
use super::reactive::Signal;
use crate::{
    Activate, AppContext, Button, Checkbox, Divider, Entity, ListItem, NodeStyle, NumberChanged,
    NumberInput, Progress, RangeChanged, RangeField, RangeInput, Select, SelectChanged,
    SelectOption, Spinner, StableNodeId, Stack, Switch, Text, TextArea, TextChanged, TextInput,
    TextSubmitted, ToggleChanged,
};

/// Components whose [`NodeStyle`] the view layer may write (visibility).
pub trait StyledComponent {
    #[doc(hidden)]
    fn node_style(&self) -> &NodeStyle;
    #[doc(hidden)]
    fn node_style_mut(&mut self) -> &mut NodeStyle;
}

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

/// Expands the control table of `nana-ui-view-schema`: per control, one
/// [`FieldWrite`] and `El` setter per field, `model` and event methods, and
/// [`StyledComponent`] for `.visible`.
macro_rules! controls {
    ($(
        $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
            $($field:ident: $ty:ty = $write:ident),* $(,)?
        }
        $(on { $($on:ident: $on_event:ident),* $(,)? })?
        $(with { $($with:ident: $with_event:ident),* $(,)? })?
        $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
        ;
    )*) => {$(
        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub mod $function {
            $(pub struct $field;)*
        }

        $(
            impl FieldWrite<$component, $ty> for $function::$field {
                const FIELD: &'static str = concat!(stringify!($component), ".", stringify!($field));

                fn write(target: &mut $component, value: $ty) {
                    controls!(@write $write target, value, $field)
                }

                fn differs(target: &$component, value: &$ty) -> bool {
                    controls!(@differs $write target, value, $field)
                }
            }
        )*

        impl<K> El<$component, K> {
            $(
                #[track_caller]
                pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                    self.prop::<$ty, $function::$field>(value)
                }
            )*

            $(
                /// `v-model`: shows the signal and writes each change back.
                #[track_caller]
                pub fn model(self, value: Signal<$model_ty>) -> Self {
                    self.$model(value)
                        .on(move |$event: &$model_event| value.set($from_event))
                }
            )?

            $($(
                /// `@` event of the template.
                pub fn $on(self, mut handler: impl FnMut() + Send + 'static) -> Self {
                    self.on(move |_: &$on_event| handler())
                }
            )*)?

            $($(
                /// `@` event of the template, with its value.
                pub fn $with(self, handler: impl FnMut(&$with_event) + Send + 'static) -> Self {
                    self.on(handler)
                }
            )*)?
        }

        impl StyledComponent for $component {
            fn node_style(&self) -> &NodeStyle {
                &self.style
            }

            fn node_style_mut(&mut self) -> &mut NodeStyle {
                &mut self.style
            }
        }
    )*};
    (@write set $target:ident, $value:ident, $field:ident) => {
        $target.$field = $value
    };
    (@differs set $target:ident, $value:ident, $field:ident) => {
        $target.$field != *$value
    };
    // Only a different value replaces the text, so echoing an edit back
    // through its signal leaves the caret and selection alone.
    (@write text_state $target:ident, $value:ident, $field:ident) => {
        if $target.state.value != $value {
            $target.state.replace_value($value);
        }
    };
    (@differs text_state $target:ident, $value:ident, $field:ident) => {
        $target.state.value != *$value
    };
    (@write assign $target:ident, $value:ident, $field:ident) => {{
        $target.assign($value);
    }};
    (@differs assign $target:ident, $value:ident, $field:ident) => {
        $target.$field() != *$value
    };
}

nana_ui_view_schema::for_each_control!(controls);

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

#[track_caller]
pub fn switch(label: impl IntoProp<String>) -> El<Switch> {
    widget(Switch::new("", false)).label(label)
}

/// A multi-line text field.
#[track_caller]
pub fn text_area() -> El<TextArea> {
    widget(TextArea::new(""))
}

#[track_caller]
pub fn number_input() -> El<NumberInput> {
    widget(NumberInput::new(0.0))
}

/// A single-value field; give it `.options(…)`.
#[track_caller]
pub fn select() -> El<Select> {
    widget(Select::new(None::<Arc<str>>))
}

#[track_caller]
pub fn list_item(label: impl IntoProp<String>) -> El<ListItem> {
    widget(ListItem::new("")).label(label)
}

/// Progress towards `max`; bind `.value(…)`.
#[track_caller]
pub fn progress(max: f64) -> El<Progress> {
    widget(Progress::new(0.0, max))
}

#[track_caller]
pub fn spinner() -> El<Spinner> {
    widget(Spinner::new(""))
}

/// A horizontal rule.
#[track_caller]
pub fn divider() -> El<Divider> {
    widget(Divider::horizontal())
}

/// A field value written as text by devtools and parsed back.
trait FieldText: Sized {
    fn parse(text: &str) -> Result<Self, String>;
}

impl FieldText for String {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(text.to_owned())
    }
}

impl FieldText for Arc<str> {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(Arc::from(text))
    }
}

impl FieldText for Option<Arc<str>> {
    /// Empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        Ok((!text.is_empty()).then(|| Arc::from(text)))
    }
}

impl FieldText for bool {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not `true` or `false`"))
    }
}

impl FieldText for f64 {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not a number"))
    }
}

impl FieldText for Vec<SelectOption> {
    fn parse(_: &str) -> Result<Self, String> {
        Err("options are edited in code, not as text".into())
    }
}

/// Every bindable field of a built-in control, as text: the control table's
/// tag and `(field, value)` pairs, `None` for anything else.
pub(crate) fn inspect_control(
    view: &dyn Any,
) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    macro_rules! inspectors {
        ($(
            $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
                $($field:ident: $ty:ty = $write:ident),* $(,)?
            }
            $(on { $($on:ident: $on_event:ident),* $(,)? })?
            $(with { $($with:ident: $with_event:ident),* $(,)? })?
            $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
            ;
        )*) => {$(
            if let Some(control) = view.downcast_ref::<$component>() {
                let _ = control;
                return Some((
                    stringify!($tag),
                    vec![$((
                        stringify!($field),
                        format!("{:?}", inspectors!(@read $write control, $field)),
                    )),*],
                ));
            }
        )*};
        (@read set $control:ident, $field:ident) => { &$control.$field };
        (@read text_state $control:ident, $field:ident) => { &$control.state.value };
        (@read assign $control:ident, $field:ident) => { $control.$field() };
    }
    nana_ui_view_schema::for_each_control!(inspectors);
    None
}

/// Write `field` of the built-in control at `node` from `text`, as its
/// binding would. `None` when the node is not a built-in control.
pub(crate) fn edit_control(
    cx: &mut AppContext,
    node: StableNodeId,
    field: &str,
    text: &str,
) -> Option<Result<(), String>> {
    macro_rules! editors {
        ($(
            $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
                $($field:ident: $ty:ty = $write:ident),* $(,)?
            }
            $(on { $($on:ident: $on_event:ident),* $(,)? })?
            $(with { $($with:ident: $with_event:ident),* $(,)? })?
            $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
            ;
        )*) => {$(
            if cx.view_is::<$component>(node) {
                return Some(match field {
                    $(stringify!($field) => <$ty as FieldText>::parse(text).and_then(|value| {
                        cx.update_component(Entity::<$component>::from_stable_id(node), |control, _| {
                            <$function::$field as FieldWrite<$component, $ty>>::write(control, value)
                        })
                        .map_err(|error| error.to_string())
                    }),)*
                    other => Err(format!(
                        "`<{}>` has no field `{other}`",
                        stringify!($tag)
                    )),
                });
            }
        )*};
    }
    nana_ui_view_schema::for_each_control!(editors);
    None
}
