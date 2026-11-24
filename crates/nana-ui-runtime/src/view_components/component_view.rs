use std::sync::Arc;

use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextInputState, TextVerticalAlignment,
    UiWorld,
};

static CONTROL_LAYOUT: std::sync::LazyLock<Arc<nana_ui_core::LayoutStyle>> =
    std::sync::LazyLock::new(|| {
        let mut theme = nana_ui_core::ThemeLayout::default();
        theme.control_typography(nana_ui_core::ControlSize::Medium);
        theme.border_width =
            Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        Arc::new(nana_ui_core::LayoutStyle {
            theme: theme.into(),
            // No `padding_left`/`padding_right` here: the control names its inset
            // step on its `NodeStyle` (`control_padding_x`), same as the two below.
            // No `min_height` here either: the control names its size step on its
            // `NodeStyle`, so the installed metrics decide how tall it is.
            border_width: Some(nana_ui_core::HAIRLINE),
            // No `border_radius` here: the tier is named on the `NodeStyle` of
            // each control that uses this layout, so the installed theme decides
            // the number. Baking it here is what made the Appearance radius
            // setting a no-op for controls.
            ..nana_ui_core::LayoutStyle::default()
        })
    });

pub(super) fn control_layout() -> Arc<nana_ui_core::LayoutStyle> {
    Arc::clone(&CONTROL_LAYOUT)
}

/// Shared page-spacing target (24px by default) for heading icon tools.
pub(crate) fn compact_icon_style(mut style: NodeStyle) -> NodeStyle {
    use nana_ui_core::LengthSpec::Px;
    style.square = None;
    style.control_height = None;
    style.control_padding_x = None;
    style.control_padding_y = None;
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(Px(24.0));
    layout.height = Some(Px(24.0));
    layout.min_width = Some(Px(24.0));
    layout.min_height = Some(Px(24.0));
    let edge = crate::theme_intent::space(nana_ui_core::SpacingStep::Page);
    layout.theme.width = Some(edge.clone());
    layout.theme.height = Some(edge.clone());
    layout.theme.min_width = Some(edge.clone());
    layout.theme.min_height = Some(edge);
    layout.padding_left = Some(Px(0.0));
    layout.padding_right = Some(Px(0.0));
    layout.padding_top = Some(Px(0.0));
    layout.padding_bottom = Some(Px(0.0));
    layout.flex_shrink = Some(0.0);
    style
}

/// A heading's labelled tool box: as tall as [`compact_icon_style`]'s square
/// (the theme's page-spacing target, 24px by default), sized across by its
/// content, with no block padding. The inline inset stays the caller's.
pub(crate) fn tool_box(mut style: NodeStyle) -> NodeStyle {
    use nana_ui_core::LengthSpec::Px;
    style.square = None;
    style.control_height = None;
    style.control_padding_y = None;
    let layout = Arc::make_mut(&mut style.layout);
    layout.height = Some(Px(24.0));
    layout.min_height = Some(Px(24.0));
    let edge = crate::theme_intent::space(nana_ui_core::SpacingStep::Page);
    layout.theme.height = Some(edge.clone());
    layout.theme.min_height = Some(edge);
    layout
        .theme
        .control_typography(nana_ui_core::ControlSize::Small);
    layout.font_size = Some(nana_ui_core::ControlSize::Small.text_size());
    layout.padding_top = Some(Px(0.0));
    layout.padding_bottom = Some(Px(0.0));
    layout.flex_shrink = Some(0.0);
    style
}

pub(crate) fn format_range_value(value: f64, step: f64) -> Arc<str> {
    let decimals = (0_i32..=6)
        .find(|decimals| {
            let scale = 10_f64.powi(*decimals);
            (step * scale - (step * scale).round()).abs() <= f64::EPSILON * scale.max(1.0)
        })
        .unwrap_or(6) as usize;
    Arc::from(format!("{value:.decimals$}"))
}

/// `value` followed by `unit`: a word (`秒`, `px`) after a space, a sign
/// (`%`, `°`) right after the number.
pub(crate) fn with_unit(value: Arc<str>, unit: Option<&str>) -> Arc<str> {
    match unit.filter(|unit| !unit.is_empty()) {
        Some(unit) if unit.chars().next().is_some_and(char::is_alphanumeric) => {
            Arc::from(format!("{value} {unit}"))
        }
        Some(unit) => Arc::from(format!("{value}{unit}")),
        None => value,
    }
}

pub(super) fn range_field_style() -> NodeStyle {
    let mut layout = nana_ui_core::LayoutStyle::default();
    layout
        .theme
        .control_typography(nana_ui_core::ControlSize::Medium);
    crate::theme_intent::weight(&mut layout, nana_ui_core::TextWeight::Medium);
    NodeStyle {
        layout: Arc::new(layout),
        background: Some(nana_ui_core::SemanticColorRole::Accent),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Faint),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        ..NodeStyle::default()
    }
}

pub(super) fn text_field_style(multiline: bool) -> NodeStyle {
    let mut layout = (*control_layout()).clone();
    layout.width = Some(nana_ui_core::LengthSpec::Percent(100.0));
    layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
    layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;
    if multiline {
        layout.theme.line_height = None;
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Relative(1.45));
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::ControlSize::Medium.height_in(nana_ui_core::UI_METRICS) * 3.0,
        ));
    } else {
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
            nana_ui_core::ControlSize::Medium.line_height(),
        ));
        layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    }
    NodeStyle {
        layout: Arc::new(layout),
        background: Some(nana_ui_core::SemanticColorRole::Background),
        border: Some(nana_ui_core::SemanticColorRole::Border),
        shape: Some(nana_ui_core::ShapeRole::Field),
        control_height: (!multiline).then_some(nana_ui_core::ControlHeight::Min(
            nana_ui_core::ControlSize::Medium,
        )),
        control_padding_x: Some(nana_ui_core::ControlPadding::Field),
        control_padding_y: multiline.then_some(nana_ui_core::ControlPadding::Field),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint::FOCUS_FIELD,
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        text_vertical_alignment: if multiline {
            TextVerticalAlignment::Top
        } else {
            TextVerticalAlignment::Center
        },
        ..NodeStyle::default()
    }
}

/// The footer is independent of the text row budget. Its action uses the
/// theme's small-control extent; a count-only footer needs one caption row.
pub(crate) fn text_area_footer_extent(
    counter: bool,
    action: bool,
    metrics: nana_ui_core::ThemeMetrics,
) -> f32 {
    if action {
        nana_ui_core::ControlSize::Small.height_in(metrics)
    } else if counter {
        nana_ui_core::ControlSize::Medium.line_height()
    } else {
        0.0
    }
}

pub(super) fn apply_input_recipe(
    style: &mut NodeStyle,
    recipe: &nana_ui_core::InputRecipe,
    multiline: bool,
    invalid: bool,
    style_override: bool,
    theme: &nana_ui_core::CompiledTheme,
) {
    if style_override {
        return;
    }
    style.background = Some(recipe.background);
    if recipe.foreground != nana_ui_core::InputRecipe::DEFAULT.foreground {
        style.foreground = Some(recipe.foreground);
    }
    style.border = Some(if invalid {
        recipe.invalid_border
    } else {
        recipe.border
    });
    style.interaction.hovered.border = Some(if invalid {
        recipe.invalid_border
    } else {
        recipe.hover_border
    });
    style.interaction.focused.border = Some(if invalid {
        recipe.invalid_border
    } else {
        recipe.focus_border
    });
    style.interaction.focused.background_mix = Some(recipe.focus_background_mix);
    style.interaction.disabled = SemanticPaint {
        foreground: Some(recipe.disabled_foreground),
        background: Some(recipe.disabled_background),
        border: Some(recipe.disabled_border),
        ..SemanticPaint::default()
    };
    if multiline {
        let layout = Arc::make_mut(&mut style.layout);
        if layout.theme.line_height.is_none()
            && (layout.line_height.is_none()
                || layout.line_height
                    == Some(nana_ui_core::LineHeightSpec::Relative(
                        nana_ui_core::InputRecipe::DEFAULT.multiline_line_height,
                    )))
        {
            layout.line_height = Some(nana_ui_core::LineHeightSpec::Relative(
                recipe.multiline_line_height,
            ));
        }
        let default_min_h = nana_ui_core::ControlSize::Medium.height_in(nana_ui_core::UI_METRICS)
            * nana_ui_core::InputRecipe::DEFAULT.multiline_height_factor;
        if layout.min_height == Some(nana_ui_core::LengthSpec::Px(default_min_h)) {
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::ControlSize::Medium.height_in(theme.metrics())
                    * recipe.multiline_height_factor,
            ));
        }
    }
}

pub(super) struct TextFieldProjection<'a> {
    pub(super) state: &'a TextInputState,
    pub(super) label: &'a Option<Arc<str>>,
    pub(super) disabled: bool,
    pub(super) busy: bool,
    pub(super) editable: bool,
    pub(super) invalid: bool,
    pub(super) multiline: bool,
    pub(super) style: &'a NodeStyle,
    pub(super) highlight: Option<&'a HighlightRequest>,
    /// The number a spinner field steps from (its parsed draft, else its
    /// committed value; see `NumberInput::step_base`) and its rules.
    pub(super) numeric: Option<(f64, nana_ui_core::NumberFieldSpec)>,
    /// Read after the value: a number field's unit, which is drawn in the
    /// field but is not part of its text.
    pub(super) description: Option<Arc<str>>,
}

pub(super) fn project_text_field(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    field: TextFieldProjection<'_>,
) {
    match world.text_input(id) {
        // Only the caret or the primary selection moved. Publishing the whole
        // state would clone the value, make the world diff it to discover it
        // did not change, and mark the node's text content dirty for a move
        // that did not touch a byte of it.
        Some(current)
            if current.value_shared() == field.state.value
                && current.additional_selections == field.state.additional_selections =>
        {
            if current.selection != field.state.selection {
                mutations.set_text_selection(id, field.state.selection);
            }
        }
        _ => mutations.set_text_input(id, Some(field.state.clone())),
    }
    if world.highlight_request(id) != field.highlight {
        mutations.set_highlight_request(id, field.highlight.cloned());
    }
    if !field.editable && world.ime(id).is_some() {
        mutations.set_ime(id, None);
    }
    project_common(
        id,
        world,
        mutations,
        field.style,
        InteractionState {
            pointer_events: !field.disabled,
            focusable: !field.disabled,
        },
        AccessibilityState {
            role: AccessibilityRole::TextInput,
            label: field.label.clone(),
            disabled: field.disabled,
            busy: field.busy,
            invalid: field.invalid,
            multiline: field.multiline,
            editable: field.editable,
            numeric_minimum: field.numeric.and_then(|(_, spec)| spec.minimum),
            numeric_maximum: field.numeric.and_then(|(_, spec)| spec.maximum),
            numeric_step: field.numeric.map(|(_, spec)| spec.effective_step()),
            numeric_value: field.numeric.map(|(value, _)| value),
            description: field.description,
            ..AccessibilityState::default()
        },
    );
}

/// A Nana-native component projects its state into the retained runtime. The
/// backend consumes the resulting UiWorld/UiScene data; no renderer type is
/// part of this contract.
///
/// `PartialEq` decides whether an [`update_component`] changed anything: a
/// component equal to what it was before the update is not projected again.
/// Every field [`Self::project`] reads must take part in the comparison.
///
/// [`update_component`]: crate::AppContext::update_component
pub trait ComponentView: Clone + PartialEq + Send + 'static {
    /// Built-in component identity (`"nana.button"`), stamped onto nodes when
    /// this view is created or mounted. Non-built-in views leave this `None`.
    const TYPE_ID: Option<&'static str> = None;

    /// What this built-in type brings with its first node; see
    /// [`crate::TypeBehavior`].
    #[doc(hidden)]
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior::NONE;

    /// Hand each layout this component holds to `share`, which may replace
    /// it with an equal one other nodes already use. Called when the
    /// component is created, so equal siblings keep one layout between them.
    #[doc(hidden)]
    fn share_layouts(
        &mut self,
        _share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
    }

    /// Hand the style this component holds as an `Arc` to `share`, which
    /// may replace it with an equal one other nodes already use; the node's
    /// record then holds the same one. Called when the component is
    /// created, after [`Self::share_layouts`].
    #[doc(hidden)]
    fn share_style(&mut self, _share: &mut dyn FnMut(&mut std::sync::Arc<NodeStyle>)) {}

    /// Bring what an application wrote inside the limits the framework's
    /// walks rely on, such as [`crate::MAX_SPLIT_DEPTH`]. Called whenever a
    /// value enters the framework: when the component is created and after
    /// every update. Defaults to nothing.
    #[doc(hidden)]
    fn fit_framework_limits(&mut self) {}

    /// Apply declarative properties to a retained component. Stateful controls
    /// override this to preserve interaction state; explicit `update_component`
    /// remains available when the caller intends to replace that state.
    fn reconcile(&mut self, next: Self) {
        *self = next;
    }

    /// Emit what the change from `previous` means, whichever write made it:
    /// the runtime's own input, an application's update, or a refresh
    /// through [`Self::reconcile`]. An event that reports state this way
    /// fires once per change and never for a write that kept the state.
    /// Runs after every update, before it is found to be a no-op. Defaults
    /// to nothing.
    #[doc(hidden)]
    fn announce_change(&self, _previous: &Self, _cx: &mut crate::ViewContext<'_, Self>) {}

    fn node_kind(&self) -> NodeKind;
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue);

    /// Opt in to one reprojection when this node's child structure changes
    /// (child insert, detach or despawn under this node, or a child shown or
    /// hidden) even though the component's own data did not change. Components that probe the retained
    /// subtree during [`Self::project`] and memoize the result into visual or
    /// text state need this to avoid stale snapshots taken before children
    /// were attached. Defaults to `false`; existing components keep their
    /// data-change-only reprojection schedule.
    fn wants_child_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Opt in to pointer-hover lifecycle tracking (enter/leave timers that
    /// drive `open`). Hover-triggered surfaces like [`crate::HoverCard`] use
    /// this so the framework can schedule their delayed open and grace close
    /// from pointer movement alone. Defaults to `false`.
    fn wants_hover_tracking() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Opt in to one reprojection when installed layout tokens change
    /// (metrics, spacing, typography, or border widths).
    ///
    /// Derived geometry that `project` spends from `height_in` / `padding_x_in`
    /// (a list's `count × row`, a segmented option's `height - 6`) is not a
    /// named layout intent, so `reresolve_layout_intent` cannot move it.
    /// Defaults to `false`.
    fn wants_metrics_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Opt in to one reprojection when the installed component recipes, or
    /// the border tokens a recipe strokes with, change.
    ///
    /// A [`ComponentRecipe`](nana_ui_core::ComponentRecipe) that this component
    /// reads in `project` — deciding which `SemanticColorRole` to author onto
    /// its own node — cannot reach the node any other way: the role lives on
    /// the authored `NodeStyle`, and only the component writes that. This is
    /// the narrow case Issue #101 §1.6 left open for reprojection, opted into
    /// per component rather than made global.
    ///
    /// Separate from [`Self::wants_metrics_reproject`] so a palette-only theme
    /// switch reprojects nothing. Defaults to `false`.
    fn wants_recipe_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Narrow theme recipe dependency mask used to filter [`Self::wants_recipe_reproject`]
    /// views during theme changes (#106).
    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask
    where
        Self: Sized,
    {
        if Self::wants_recipe_reproject() {
            nana_ui_core::ThemeDependencyMask::RECIPE_ANY
        } else {
            nana_ui_core::ThemeDependencyMask::NONE
        }
    }

    /// Opt in to one reprojection when the reduce-motion preference
    /// ([`crate::AppContext::reduced_motion`]) changes.
    ///
    /// For a component whose projection starts or stops a timeline of its
    /// own by it, or draws differently while motion is reduced: nothing
    /// else about it changed, so nothing else would project it again.
    /// Defaults to `false`.
    fn wants_reduced_motion_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Opt in to one reprojection when the host's text backend changes — the
    /// first engine a world is shaped through included.
    ///
    /// For geometry `project` spends from a text measurement (an editor's
    /// line-number gutter): a component is projected before any host has
    /// shaped, so without this its first measurement is the estimate and
    /// stays one until its data next changes. Defaults to `false`.
    fn wants_text_backend_reproject() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Whether [`Self::project`] depends on something `PartialEq` cannot see
    /// change, so an update that leaves the component equal must still
    /// project it:
    ///
    /// - state shared behind interior mutability, which a clone shares;
    /// - styles it patches onto nodes other components own and project (a
    ///   container making the content it hosts fill its box), which their
    ///   own projections overwrite.
    ///
    /// The update still returns early when that projection writes nothing.
    const ALWAYS_REPROJECT: bool = false;
}

/// `text` as an `Arc<str>`, copied at most once per projection.
pub(super) fn shared_label(shared: &mut Option<Arc<str>>, text: &str) -> Arc<str> {
    Arc::clone(shared.get_or_insert_with(|| Arc::from(text)))
}

/// Imperative builders replace all edges; raw LayoutStyle keeps CSS precedence.
pub(super) fn replace_padding_xy(layout: &mut nana_ui_core::LayoutStyle, x: f32, y: f32) {
    use nana_ui_core::LengthSpec;
    layout.padding = None;
    if !layout.logical_edges.is_shared_default() {
        layout.logical_edges.padding = Default::default();
    }
    layout.padding_left = Some(LengthSpec::Px(x.max(0.0)));
    layout.padding_right = layout.padding_left;
    layout.padding_top = Some(LengthSpec::Px(y.max(0.0)));
    layout.padding_bottom = layout.padding_top;
}

pub(crate) fn project_common(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    interaction: InteractionState,
    accessibility: AccessibilityState,
) {
    if world.node_style(id) != Some(style) {
        mutations.set_style(id, style.clone());
    }
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
    if !world.accessibility_eq(id, &accessibility) {
        mutations.set_accessibility(id, accessibility);
    }
}
