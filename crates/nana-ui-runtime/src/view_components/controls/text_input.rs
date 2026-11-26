#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};
use crate::{AppContext, Entity};

#[derive(Debug, Clone, PartialEq)]
pub struct TextInput {
    pub state: TextInputState,
    pub max_length: Option<usize>,
    /// Show committed Unicode scalar count and this display-only limit.
    pub character_counter: Option<usize>,
    pub label: Option<Arc<str>>,
    /// Said after the name by assistive technology (`aria-description`),
    /// never drawn. `None` says nothing more.
    pub accessible_description: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub read_only: bool,
    pub secure: bool,
    pub invalid: bool,
    /// Clear focus from this field when Enter submits (`TextSubmitted`).
    pub blur_on_submit: bool,
    /// Focus this field in place and select all text when it is mounted into
    /// a visible subtree (used by inline-edit / rename fields).
    pub focus_and_select_on_mount: bool,
    pub style: NodeStyle,
    pub highlight: Option<HighlightRequest>,
    pub(crate) style_override: bool,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(Into::<String>::into(value)),
            max_length: None,
            character_counter: None,
            label: None,
            accessible_description: None,
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            loading: false,
            read_only: false,
            secure: false,
            invalid: false,
            blur_on_submit: false,
            focus_and_select_on_mount: false,
            style: text_field_style(false),
            highlight: None,
            style_override: false,
        }
    }

    /// Color committed text with the registered `"highlight"` presenter.
    pub fn highlight(mut self, language: impl Into<Arc<str>>) -> Self {
        self.highlight = Some(HighlightRequest::highlight(language));
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spoken name (`aria-label`); the same field as [`Self::label`],
    /// which this control never draws. Independent of the placeholder. An
    /// empty name is none.
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        self.label = (!name.is_empty()).then_some(name);
        self
    }

    /// See [`Self::accessible_description`](field@Self::accessible_description);
    /// an empty description is none.
    pub fn accessible_description(mut self, description: impl Into<Arc<str>>) -> Self {
        let description = description.into();
        self.accessible_description = (!description.is_empty()).then_some(description);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Replace host-owned outer layout while retaining TextInput semantic
    /// paint and interaction states.
    /// Supply the whole box. Design intent that writes into the box
    /// (`control_height`, `control_padding_x`, `radius`, `shape`) is dropped:
    /// replacing the layout used to wipe those defaults when they were baked
    /// into it, and a caller who hands over a complete box owns its height,
    /// inset and corners.
    pub fn layout(mut self, layout: Arc<nana_ui_core::LayoutStyle>) -> Self {
        self.style.layout = layout;
        self.style.control_height = None;
        self.style.control_padding_x = None;
        self.style.clear_corners();
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        // The step, not the pixels: the installed metrics decide the height.
        self.style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
        self.style.control_padding_x = Some(size.into());
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(size.text_size());
        layout.theme.control_typography(size);
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// Clear focus from this field after Enter emits [`TextSubmitted`].
    pub fn blur_on_submit(mut self, blur: bool) -> Self {
        self.blur_on_submit = blur;
        self
    }

    /// Focus this field in place and select all its text once mounted into a
    /// visible subtree.
    pub fn focus_and_select_on_mount(mut self, enabled: bool) -> Self {
        self.focus_and_select_on_mount = enabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }

    /// Limit user edits to UTF-16 code units, as HTML `maxlength` does.
    /// Programmatic initial/state values are preserved even when longer.
    /// Existing over-limit values may be shortened or replaced without growth.
    /// IME preedit is unrestricted; only its committed text is limited.
    pub fn max_length(mut self, limit: usize) -> Self {
        self.max_length = Some(limit);
        self
    }

    /// Display `count/limit` inside the trailing edge of the field.
    /// Counts committed Unicode scalar values (`chars`), excluding IME preedit.
    /// This does not restrict edits or submission; `max_length` is independent.
    pub fn character_counter(mut self, limit: usize) -> Self {
        self.character_counter = Some(limit);
        self
    }

    pub(crate) fn accepts_edit_value(&self, value: &str) -> bool {
        crate::text_input_limit::accepts_value(&self.state.value, value, self.max_length)
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        crate::text_input_limit::replace(&mut self.state, text, self.max_length, false)
    }

    pub(crate) fn commit_limited_ime(&mut self, text: &str) -> bool {
        crate::text_input_limit::replace(&mut self.state, text, self.max_length, true)
    }
}

impl ComponentView for TextInput {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TEXT_INPUT.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        editable: Some(&crate::framework::Editable::<Self>::EDITOR_HOOKS),
        pointer_ops: Some(&crate::framework::PointerOps {
            close_text_atom: Some(AppContext::close_focused_text_atom),
            ..TEXT_INPUT_EMPTY_POINTER_OPS
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Equal host values preserve the editing selection. A different value uses
    /// the supplied selection (`TextInput::new` places its caret at the end).
    fn reconcile(&mut self, mut next: Self) {
        if self.state.value == next.state.value {
            next.state = self.state.clone();
        }
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "input".into(),
        }
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_INPUT
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::TextInputVisual {
            placeholder: Arc::clone(&self.placeholder),
            size: self.size,
            secure: self.secure,
            invalid: self.invalid,
            steppers: false,
            character_counter: self.character_counter,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            color_swatches: Arc::from([]),
            atoms: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            git_marks: Arc::from([]),
            editor_options: TextEditorRenderOptions::default(),
        };
        if world.text_input_visual_ref(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let node_theme = world.node_theme(id);
        let mut effective_style = self.style.clone();
        apply_input_recipe(
            &mut effective_style,
            node_theme.recipes().input(),
            false,
            self.invalid,
            self.style_override,
            node_theme,
        );
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled || self.loading,
                busy: self.loading,
                editable: !self.disabled && !self.loading && !self.read_only,
                invalid: self.invalid,
                multiline: false,
                style: &effective_style,
                highlight: self.highlight.as_ref(),
                numeric: None,
                description: self.accessible_description.clone(),
            },
        );
    }
}

/// Numeric spinner: a text field that also steps.
///
/// `value` is the committed authority; `state` carries the in-progress draft
/// while the user types. Typing never rewrites `value` — the draft is parsed on
/// commit (Enter or blur), and an unparseable draft restores the last committed
/// value instead of inventing one. Stepping and arrow keys start from the
/// draft when it parses, so a typed number is stepped rather than discarded;
/// a half-typed draft that does not parse steps from `value` instead.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberInput {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub spec: nana_ui_core::NumberFieldSpec,
    pub size: nana_ui_core::ControlSize,
    /// Show the trailing increment/decrement buttons. Keyboard stepping remains available.
    pub steppers: bool,
    /// The unit after the number ("ms", "%"), drawn in the field before the
    /// steppers; see [`crate::TextEditorRenderOptions::suffix`]. It is not
    /// part of the text: typing, parsing and the committed value never see it.
    pub unit: Option<Arc<str>>,
    pub disabled: bool,
    pub read_only: bool,
    pub invalid: bool,
    pub style: NodeStyle,
    pub(crate) value: f64,
    /// The number the application asked for, before the field's rules
    /// applied. Each builder call normalizes it again against the rules so
    /// far, so the finished rules decide: `new(0.0).range(0.1, 5.0)` then
    /// `.precision(1)` starts at 0.1, not at the 1 a whole-number grid made of
    /// it halfway through the chain.
    pub(crate) requested: f64,
    pub(crate) continuous: bool,
    pub(crate) style_override: bool,
}

impl NumberInput {
    pub fn new(value: f64) -> Self {
        let spec = nana_ui_core::NumberFieldSpec::default();
        let requested = value;
        let value = spec.snap(value);
        Self {
            state: TextInputState::new(spec.format(value)),
            label: None,
            placeholder: Arc::from(""),
            spec,
            size: nana_ui_core::ControlSize::Medium,
            steppers: true,
            unit: None,
            disabled: false,
            read_only: false,
            invalid: false,
            style: text_field_style(false),
            value,
            requested,
            continuous: false,
            style_override: false,
        }
    }

    /// A continuous numeric field. Committed values are clamped to bounds but
    /// never rounded to the step grid; `step` only controls spinner/arrow
    /// increments. Text uses the shortest round-tripping representation, so
    /// `precision` does not discard fractional values in this mode.
    pub fn continuous(value: f64) -> Self {
        let mut field = Self::new(0.0);
        field.continuous = true;
        field.assign(value);
        field
    }

    pub fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.spec.minimum = Some(minimum);
        self.spec.maximum = Some(maximum);
        self.resync();
        self
    }

    pub fn minimum(mut self, minimum: f64) -> Self {
        self.spec.minimum = Some(minimum);
        self.resync();
        self
    }

    pub fn maximum(mut self, maximum: f64) -> Self {
        self.spec.maximum = Some(maximum);
        self.resync();
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.spec.step = step;
        self.resync();
        self
    }

    pub fn precision(mut self, precision: u8) -> Self {
        self.spec.precision = precision;
        self.resync();
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        self.style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
        self.style.control_padding_x = Some(size.into());
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(size.text_size());
        layout.theme.control_typography(size);
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }

    /// Draw `unit` after the number; see [`Self::unit`].
    pub fn unit(mut self, unit: impl Into<Arc<str>>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Hide the pointer steppers without changing numeric editing or arrow keys.
    pub fn steppers(mut self, steppers: bool) -> Self {
        self.steppers = steppers;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }

    /// Last committed value.
    pub const fn value(&self) -> f64 {
        self.value
    }

    pub(crate) fn accepts_input(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// Publish a value from the application. Values are clamped to bounds;
    /// discrete fields also snap them to their precision and step grid.
    pub fn assign(&mut self, value: f64) -> bool {
        self.requested = value;
        let next = self.normalize(value);
        let text = if self.continuous {
            next.to_string()
        } else {
            self.spec.format(next)
        };
        if next == self.value && self.state.value == text {
            return false;
        }
        self.value = next;
        self.state.replace_value(text);
        true
    }

    /// Move by grid positions from the typed draft when it parses, from the
    /// committed value otherwise.
    ///
    /// A step that cannot move away from its base (a bound) changes nothing,
    /// matching the spinner half drawn inert there.
    pub(crate) fn step_value(&mut self, steps: i32) -> bool {
        let base = self.step_base();
        let next = if self.continuous {
            self.spec
                .clamp(base + f64::from(steps) * self.spec.effective_step())
        } else {
            self.spec.step_by(base, steps)
        };
        // Compare where the field would put it: a bound off the grid (10.3
        // on a whole-number grid) snaps a step back onto its base.
        if self.normalize(next) == base {
            return false;
        }
        self.assign(next)
    }

    /// The number a step starts from: the typed draft when it parses, the
    /// committed value otherwise. It is also the number the field publishes,
    /// so the spinner's enabled halves agree with what a step would do.
    ///
    /// Normalized as a commit would normalize it: a continuous draft such as
    /// `500` over a maximum of 100 is 100 here, not 500.
    pub(crate) fn step_base(&self) -> f64 {
        nana_ui_core::NumberFieldSpec::parse_unsnapped(&self.state.value)
            .map_or(self.value, |parsed| self.normalize(parsed))
    }

    /// Whether `value` lies within the field's bounds, so the field can take
    /// it without clamping (snapping to the grid aside), allowing each bound
    /// its float noise.
    pub(crate) fn within_bounds(&self, value: f64) -> bool {
        self.spec.within_bounds(value)
    }

    /// Put back a committed number the undo journal recorded, leaving the
    /// draft as the journal restored it. The number is normalized against
    /// the field's current bounds and grid, which may have changed since it
    /// was recorded. Returns whether the number moved.
    pub(crate) fn restore_value(&mut self, value: f64) -> bool {
        let value = self.normalize(value);
        self.requested = value;
        if value == self.value {
            return false;
        }
        self.value = value;
        true
    }

    /// Publish a number from the application: a number the field already
    /// holds changes nothing, so a draft the user is typing survives a
    /// controlled field echoing its own value back. Otherwise as
    /// [`Self::assign`].
    pub(crate) fn publish(&mut self, value: f64) -> bool {
        if self.normalize(value) == self.value {
            return false;
        }
        self.assign(value)
    }

    /// The field's rules with its bounds brought onto its grid: the numbers
    /// it can actually reach. A maximum of 10 on a grid of 3 is 9. Published
    /// for assistive technology and the spinner, so a half is drawn inert
    /// exactly where a step can no longer move. A discrete field publishes
    /// the step it moves by, never finer than its precision displays.
    fn reachable_spec(&self) -> nana_ui_core::NumberFieldSpec {
        nana_ui_core::NumberFieldSpec {
            step: if self.continuous {
                self.spec.step
            } else {
                self.spec.display_step()
            },
            // A non-finite bound is no bound, as the spec itself reads it.
            minimum: self
                .spec
                .minimum
                .filter(|minimum| minimum.is_finite())
                .map(|minimum| self.normalize(minimum)),
            maximum: self
                .spec
                .maximum
                .filter(|maximum| maximum.is_finite())
                .map(|maximum| self.normalize(maximum)),
            ..self.spec
        }
    }

    /// Where the field puts `value`: clamped to its bounds, and on a
    /// discrete field also snapped to its precision and step grid.
    fn normalize(&self, value: f64) -> f64 {
        if self.continuous {
            self.spec.clamp(value)
        } else {
            self.spec.snap(value)
        }
    }

    /// Parse the draft. An unparseable draft restores the committed value.
    pub(crate) fn commit_draft(&mut self) -> bool {
        match nana_ui_core::NumberFieldSpec::parse_unsnapped(&self.state.value) {
            Some(parsed) => self.assign(parsed),
            None => self.revert_draft(),
        }
    }

    /// Show the committed value again, discarding the draft. Returns whether
    /// the draft differed.
    pub(crate) fn revert_draft(&mut self) -> bool {
        let restored = self.formatted_value();
        if self.state.value == restored {
            return false;
        }
        self.state.replace_value(restored);
        true
    }

    fn resync(&mut self) {
        self.assign(self.requested);
    }

    pub(crate) fn formatted_value(&self) -> String {
        if self.continuous {
            self.value.to_string()
        } else {
            self.spec.format(self.value)
        }
    }
}

const TEXT_INPUT_EMPTY_POINTER_OPS: crate::framework::PointerOps =
    crate::framework::PointerOps::EMPTY;

pub(crate) fn install_number_input_capabilities(caps: &mut crate::framework::Capabilities) {
    caps.number_step = Some(AppContext::step_focused_number_input);
    caps.number_submit = Some(AppContext::submit_focused_number_input);
    caps.add_edit_canceller(AppContext::cancel_focused_number_input);
}

static NUMBER_INPUT_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    skip_on_activation_click: true,
    releases_pointer_on_press: true,
    press: Some(|cx, _doc, id, _ptr, button, x, y| {
        if button == 0 {
            cx.press_number_stepper(id, x, y)
        } else {
            Ok(false)
        }
    }),
    select_on_focus_entry: Some(AppContext::select_focused_number_on_entry),
    ..TEXT_INPUT_EMPTY_POINTER_OPS
};

static NUMBER_INPUT_A11Y: crate::framework::A11yBehavior = crate::framework::A11yBehavior {
    step: Some(|cx, id, steps| cx.step_number_input(Entity::from_stable_id(id), steps)),
    set_text: Some(|cx, id, text| cx.set_number_text(Entity::from_stable_id(id), text)),
    activate_context_menu_index: None,
    focus_context_menu_index: None,
    activate_select_option: None,
    adjust_range: None,
    set_expanded: None,
    set_range_value: None,
};

static NUMBER_INPUT_FOCUS: crate::framework::FocusBehavior = crate::framework::FocusBehavior {
    on_blur_settle: Some(AppContext::settle_focused_number_input),
    on_focus_lost: None,
};

impl ComponentView for NumberInput {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::NUMBER_INPUT.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(install_number_input_capabilities),
        editable: Some(&crate::framework::Editable::<Self>::EDITOR_HOOKS),
        pointer_ops: Some(&NUMBER_INPUT_POINTER_OPS),
        a11y: Some(&NUMBER_INPUT_A11Y),
        focus: Some(&NUMBER_INPUT_FOCUS),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "number-input".into(),
        }
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_INPUT
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::TextInputVisual {
            placeholder: Arc::clone(&self.placeholder),
            size: self.size,
            secure: false,
            invalid: self.invalid,
            steppers: self.steppers,
            character_counter: None,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            color_swatches: Arc::from([]),
            atoms: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            git_marks: Arc::from([]),
            editor_options: TextEditorRenderOptions {
                suffix: self.unit.clone(),
                ..TextEditorRenderOptions::default()
            },
        };
        if world.text_input_visual_ref(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let node_theme = world.node_theme(id);
        let mut effective_style = self.style.clone();
        apply_input_recipe(
            &mut effective_style,
            node_theme.recipes().input(),
            false,
            self.invalid,
            self.style_override,
            node_theme,
        );
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled,
                busy: false,
                editable: self.accepts_input(),
                invalid: self.invalid,
                multiline: false,
                style: &effective_style,
                highlight: None,
                numeric: Some((self.step_base(), self.reachable_spec())),
                description: self.unit.clone(),
            },
        );
    }
}
