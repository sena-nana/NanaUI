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
//!     field: FieldType = write,
//!     …
//! }
//! [on { on_event_name: EventType, … }]        // handlers take no argument
//! [with { on_event_name: EventType, … }]      // handlers take `&EventType`
//! [model field: FieldType => EventType |event| value_from_event]
//! ;
//! ```
//!
//! `write` says how a value lands in the component:
//!
//! - `set`: the field of that name;
//! - `text_state`: the edited text, replaced only when it differs, so an
//!   edit echoed back keeps the caret;
//! - `assign`: a number field's value through `assign`; a new value puts the
//!   caret after it, as a field built at that value has it;
//! - `clamp`: a range's value, held inside its range (a value the data does
//!   not have yet, or a non-finite one, lands on the minimum);
//! - `span_low` / `span_high`: one thumb of a range span, held inside its
//!   range and pushing the other thumb along instead of crossing it, so the
//!   order two bindings land in does not change the pair;
//! - `span`: both thumbs of a range span as `(low, high)`, in order whichever
//!   way they arrive;
//! - `builder`: through the component's builder method of the same name,
//!   which keeps what it derives (a colour's hue, saturation and value);
//! - `tab`: the chosen tab, which also takes the strip's focus;
//! - `resource`: an image's host texture slot, ready while it names one.
//!
//! An argument's kind is `text` (a string: the attribute, or the element's
//! one text child), `f32` / `f64` (a number attribute, literals typed) or
//! `expr` (any Rust expression, such as an icon). A `text` argument is also
//! the field of the same name.

/// Call `$callback!` with the control table; see the crate docs.
#[macro_export]
macro_rules! for_each_control {
    ($callback:path) => {
        $callback! {
            Text => text(value: text) for Text {
                value: String = set,
                decorative: bool = set,
            };
            Button => button(label: text) for Button {
                label: String = set,
                accessible_name: String = set,
                disabled: bool = set,
                loading: bool = set,
                kind: ButtonKind = set,
            }
            on { on_activate: Activate };
            Checkbox => checkbox(label: text) for Checkbox {
                label: String = set,
                checked: bool = set,
                disabled: bool = set,
            }
            with { on_change: ToggleChanged }
            model checked: bool => ToggleChanged |event| event.checked;
            Switch => switch(label: text) for Switch {
                label: String = set,
                checked: bool = set,
                disabled: bool = set,
                loading: bool = set,
            }
            with { on_change: ToggleChanged }
            model checked: bool => ToggleChanged |event| event.checked;
            Slider => slider(min: f64, max: f64, step: f64) for RangeField {
                value: f64 = clamp,
                label: Option<Arc<str>> = set,
                show_label: bool = set,
                disabled: bool = set,
            }
            with { on_input: RangeInput, on_change: RangeChanged }
            model value: f64 => RangeInput |event| event.value;
            RangeSpan => range_span(min: f64, max: f64, step: f64) for RangeSpanField {
                low: f64 = span_low,
                high: f64 = span_high,
                span: (f64, f64) = span,
                indicator: Option<f64> = set,
                label: Option<Arc<str>> = set,
                low_label: Option<Arc<str>> = set,
                high_label: Option<Arc<str>> = set,
                orientation: RangeSpanOrientation = set,
                disabled: bool = set,
            }
            with { on_input: RangeSpanInput, on_change: RangeSpanChanged }
            model span: (f64, f64) => RangeSpanInput |event| (event.low, event.high);
            TextInput => text_input() for TextInput {
                value: String = text_state,
                label: Option<Arc<str>> = set,
                placeholder: Arc<str> = set,
                disabled: bool = set,
            }
            with { on_input: TextChanged, on_submit: TextSubmitted }
            model value: String => TextChanged |event| event.value.to_string();
            TextArea => text_area() for TextArea {
                value: String = text_state,
                label: Option<Arc<str>> = set,
                placeholder: Arc<str> = set,
                disabled: bool = set,
                read_only: bool = set,
            }
            with { on_input: TextChanged }
            model value: String => TextChanged |event| event.value.to_string();
            NumberInput => number_input() for NumberInput {
                value: f64 = assign,
                label: Option<Arc<str>> = set,
                placeholder: Arc<str> = set,
                disabled: bool = set,
                read_only: bool = set,
            }
            with { on_change: NumberChanged }
            model value: f64 => NumberChanged |event| event.value;
            Select => select() for Select {
                value: Option<Arc<str>> = set,
                options: Vec<SelectOption> = set,
                label: Option<Arc<str>> = set,
                placeholder: Option<Arc<str>> = set,
                disabled: bool = set,
                loading: bool = set,
                fit_options: bool = set,
            }
            with { on_change: SelectChanged }
            model value: Option<Arc<str>> => SelectChanged |event| Some(event.value.clone());
            ListItem => list_item(label: text) for ListItem {
                label: String = set,
                detail: String = set,
                selected: bool = set,
                disabled: bool = set,
                role: ListItemRole = set,
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
            Thumbnail => thumbnail() for Thumbnail {
                resource: Arc<str> = resource,
                generation: u64 = set,
                version: u64 = set,
                aspect: f32 = set,
                label: Arc<str> = set,
                decorative: bool = set,
            };
            Avatar => avatar(size: f32) for Avatar {
                resource: Arc<str> = set,
                generation: u64 = set,
                version: u64 = set,
                size: f32 = set,
                label: Arc<str> = set,
            };
            Texture => texture() for GpuTextureView {
                resource: Arc<str> = set,
                generation: u64 = set,
                version: u64 = set,
                opacity: f32 = set,
                corner_radius: f32 = set,
            };
            IconButton => icon_button(icon: expr, label: text) for IconButton {
                icon: Icon = set,
                label: Arc<str> = set,
                selected: bool = set,
                disabled: bool = set,
            }
            on { on_activate: Activate };
            Chip => chip(label: text) for Chip {
                label: Arc<str> = set,
                selected: bool = set,
                disabled: bool = set,
            }
            on { on_activate: Activate };
            StatusBadge => status_badge(label: text) for StatusBadge {
                label: Arc<str> = set,
                tone: StatusTone = set,
                compact: bool = set,
            };
            EmptyState => empty_state(title: text) for EmptyState {
                title: Arc<str> = set,
                message: Option<Arc<str>> = set,
                icon: Option<Icon> = set,
                compact: bool = set,
            };
            LabeledValue => labeled_value(label: text) for LabeledValue {
                label: Arc<str> = set,
                value: Arc<str> = set,
                compact: bool = set,
            };
            Tabs => tabs() for Tabs {
                selected: Arc<str> = tab,
                label: Option<Arc<str>> = set,
            }
            with { on_change: TabsEvent };
            TreeView => tree_view() for TreeView {
                nodes: Vec<TreeNode<Arc<str>>> = set,
            };
            ColorField => color_field() for ColorField {
                value: [f32; 4] = builder,
                label: Option<Arc<str>> = set,
                disabled: bool = set,
            }
            with { on_change: ColorChanged };
            ActionMenuItem => action_menu_item(label: text) for ActionMenuItem {
                label: Arc<str> = set,
                accessible_name: Arc<str> = set,
                disabled: bool = set,
                danger: bool = set,
                active: bool = set,
            }
            on { on_activate: Activate };
        }
    };
}
