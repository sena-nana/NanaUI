//! The one description of the built-in controls the declarative view layer
//! knows by name: their template tag, the element function a template calls
//! and its arguments, their bindable fields, `v-model` and events.
//!
//! `nana-ui-runtime` expands it into typed setters, `model` and event
//! methods; `nana-ui-view-codegen` (behind `view!` and the `.vue` compiler)
//! into the tags and attributes it accepts, so a misspelled attribute is a
//! template error at its line, and a control added here gains both at once.
//!
//! [`for_each_control!`] calls back a macro with the table:
//!
//! ```text
//! Tag => element_function(argument: kind, …) for RustType {
//!     field: FieldType = write,      // write: set | text_state | assign
//!     …
//! }
//! [on { on_event_name: EventType, … }]
//! [model field: FieldType => EventType |event| value_from_event]
//! ;
//! ```
//!
//! An argument's kind is `text` (a string: the attribute, or the element's
//! one text child) or `f32` / `f64` (a number attribute, literals typed). A
//! `text` argument is also the field of the same name.

/// Call `$callback!` with the control table; see the crate docs.
#[macro_export]
macro_rules! for_each_control {
    ($callback:path) => {
        $callback! {
            Text => text(value: text) for Text {
                value: String = set,
            };
            Button => button(label: text) for Button {
                label: String = set,
                disabled: bool = set,
                loading: bool = set,
            }
            on { on_activate: Activate };
            Checkbox => checkbox(label: text) for Checkbox {
                label: String = set,
                checked: bool = set,
                disabled: bool = set,
            }
            model checked: bool => ToggleChanged |event| event.checked;
            Switch => switch(label: text) for Switch {
                label: String = set,
                checked: bool = set,
                disabled: bool = set,
                loading: bool = set,
            }
            model checked: bool => ToggleChanged |event| event.checked;
            Slider => slider(min: f64, max: f64, step: f64) for RangeField {
                value: f64 = set,
                disabled: bool = set,
            }
            model value: f64 => RangeInput |event| event.value;
            TextInput => text_input() for TextInput {
                value: String = text_state,
                placeholder: Arc<str> = set,
                disabled: bool = set,
            }
            model value: String => TextChanged |event| event.value.to_string();
            TextArea => text_area() for TextArea {
                value: String = text_state,
                placeholder: Arc<str> = set,
                disabled: bool = set,
                read_only: bool = set,
            }
            model value: String => TextChanged |event| event.value.to_string();
            NumberInput => number_input() for NumberInput {
                value: f64 = assign,
                placeholder: Arc<str> = set,
                disabled: bool = set,
                read_only: bool = set,
            }
            model value: f64 => NumberChanged |event| event.value;
            Select => select() for Select {
                value: Option<Arc<str>> = set,
                options: Vec<SelectOption> = set,
                placeholder: Option<Arc<str>> = set,
                disabled: bool = set,
                loading: bool = set,
            }
            model value: Option<Arc<str>> => SelectChanged |event| Some(event.value.clone());
            ListItem => list_item(label: text) for ListItem {
                label: String = set,
                detail: String = set,
                selected: bool = set,
                disabled: bool = set,
            }
            on { on_activate: Activate };
            Progress => progress(max: f64) for Progress {
                value: f64 = set,
                label: Option<Arc<str>> = set,
            };
            Spinner => spinner() for Spinner {
                label: Arc<str> = set,
            };
            Divider => divider() for Divider {};
        }
    };
}
