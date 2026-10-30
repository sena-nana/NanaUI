use std::sync::Arc;
use std::time::Duration;

use crate::{
    AccessibilityRole, AccessibilityState, AlignSpec, AnimationDirection, AnimationFillMode,
    AnimationIteration, AnimationPlayState, AnimationPlayback, AnimationSpec, ComponentView,
    Easing, FlexDirection, InteractionState, JustifySpec, LengthSpec, MutationQueue, NodeKind,
    NodeStyle, SemanticColorRole, StableNodeId, StandardVisual, TextContent, TextVerticalAlignment,
    UiWorld, component_animation_id, component_animation_kinds,
};
use nana_ui_core::Icon;

fn project_visual(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    text: &str,
    style: NodeStyle,
    visual: StandardVisual,
    accessibility: AccessibilityState,
) {
    if world.text(id) != Some(text) {
        mutations.set_text(
            id,
            TextContent {
                value: text.to_owned().into(),
            },
        );
    }
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
    let pointer_events = matches!(&visual, StandardVisual::LabeledValue { value_is_path: true, value, .. } if !value.is_empty());
    if world.standard_visual(id) != Some(visual.clone()) {
        mutations.set_standard_visual(id, Some(visual));
    }
    let interaction = InteractionState {
        pointer_events,
        focusable: false,
    };
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
    project_accessibility(id, world, mutations, accessibility);
}

fn project_accessibility(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    accessibility: AccessibilityState,
) {
    if world.accessibility(id) != Some(&accessibility) {
        mutations.set_accessibility(id, accessibility);
    }
}

fn status_foreground(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    crate::components::status_tone_role(tone)
}

fn status_background(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    match tone {
        nana_ui_core::StatusTone::Neutral => SemanticColorRole::Subtle,
        nana_ui_core::StatusTone::Info => SemanticColorRole::AccentSoft,
        nana_ui_core::StatusTone::Success => SemanticColorRole::Subtle,
        nana_ui_core::StatusTone::Warning => SemanticColorRole::WarningSoft,
        nana_ui_core::StatusTone::Danger => SemanticColorRole::Subtle,
    }
}

/// Compact semantic status text. It is descriptive and never owns an action.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBadge {
    pub label: Arc<str>,
    pub tone: nana_ui_core::StatusTone,
    pub compact: bool,
    pub style: NodeStyle,
}

impl StatusBadge {
    pub fn new(label: impl Into<Arc<str>>, tone: nana_ui_core::StatusTone) -> Self {
        Self {
            label: label.into(),
            tone,
            compact: true,
            style: NodeStyle::default(),
        }
    }

    pub fn tone(mut self, tone: nana_ui_core::StatusTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(status_foreground(self.tone));
        style.background = Some(status_background(self.tone));
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        // The dot and its gap are content parts (#205); the padding is the
        // pill's own inset on both sides.
        let (horizontal, vertical) = if self.compact {
            (nana_ui_core::space::SM, nana_ui_core::space::XXS)
        } else {
            (nana_ui_core::space::MD, nana_ui_core::space::XS)
        };
        layout.padding_left = Some(LengthSpec::Px(horizontal));
        layout.padding_right = Some(LengthSpec::Px(horizontal));
        use nana_ui_core::SpacingStep;
        let horizontal_step = if self.compact {
            SpacingStep::Sm
        } else {
            SpacingStep::Md
        };
        let vertical_step = if self.compact {
            SpacingStep::Xxs
        } else {
            SpacingStep::Xs
        };
        layout.theme.padding_left = Some(crate::theme_intent::space(horizontal_step));
        layout.theme.padding_right = Some(crate::theme_intent::space(horizontal_step));
        layout.theme.padding_top = Some(crate::theme_intent::space(vertical_step));
        layout.theme.padding_bottom = Some(crate::theme_intent::space(vertical_step));
        layout.padding_top = Some(LengthSpec::Px(vertical));
        layout.padding_bottom = Some(LengthSpec::Px(vertical));
        layout.border_width = Some(0.0);
        layout.border_radius = Some(999.0);
        layout.font_size = Some(if self.compact {
            nana_ui_core::type_scale::HINT
        } else {
            nana_ui_core::type_scale::META
        });
        crate::theme_intent::font(
            layout,
            if self.compact {
                nana_ui_core::TypeRole::Hint
            } else {
                nana_ui_core::TypeRole::Meta
            },
        );
        layout.font_weight = Some(nana_ui_core::type_scale::MEDIUM);
        crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Medium);
        style
    }
}

impl ComponentView for StatusBadge {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "status-badge".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            self.label.as_ref(),
            self.effective_style(),
            StandardVisual::StatusBadge {
                label: Arc::clone(&self.label),
                tone: self.tone,
                compact: self.compact,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Inline validation feedback whose semantic intent is available to every backend.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationMessage {
    pub message: Arc<str>,
    pub intent: nana_ui_core::ValidationIntent,
    pub compact: bool,
    pub style: NodeStyle,
}

impl ValidationMessage {
    pub fn new(message: impl Into<Arc<str>>, intent: nana_ui_core::ValidationIntent) -> Self {
        Self {
            message: message.into(),
            intent,
            compact: true,
            style: NodeStyle::default(),
        }
    }

    pub fn intent(mut self, intent: nana_ui_core::ValidationIntent) -> Self {
        self.intent = intent;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(match self.intent {
            nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
            nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
        });
        let layout = Arc::make_mut(&mut style.layout);
        // The dot and its gap are content parts (#205); the message keeps
        // no left inset of its own.
        layout.padding_left = Some(LengthSpec::Px(0.0));
        layout.theme.padding_left = None;
        layout.font_size = Some(if self.compact {
            nana_ui_core::type_scale::HINT
        } else {
            nana_ui_core::type_scale::META
        });
        crate::theme_intent::font(
            layout,
            if self.compact {
                nana_ui_core::TypeRole::Hint
            } else {
                nana_ui_core::TypeRole::Meta
            },
        );
        layout.font_weight = None;
        style
    }
}

impl ComponentView for ValidationMessage {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "validation-message".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            self.message.as_ref(),
            self.effective_style(),
            StandardVisual::ValidationMessage {
                message: Arc::clone(&self.message),
                intent: self.intent,
                compact: self.compact,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: Some(Arc::clone(&self.message)),
                invalid: true,
                ..AccessibilityState::default()
            },
        );
    }
}

/// A flat feedback surface with intrinsic semantic content and an optional
/// application-owned action child.
#[derive(Debug, Clone, PartialEq)]
pub struct EmptyState {
    pub title: Arc<str>,
    pub message: Option<Arc<str>>,
    pub icon: Option<Icon>,
    pub compact: bool,
    pub action: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl EmptyState {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            message: None,
            icon: None,
            compact: false,
            action: None,
            style: NodeStyle::default(),
        }
    }

    pub fn message(mut self, message: impl Into<Arc<str>>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Declare an action child identity. Mount it through
    /// [`crate::AppContext::set_empty_state_action`].
    pub fn action_child(mut self, action: StableNodeId) -> Self {
        self.action = Some(action);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = None;
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = if self.compact {
            AlignSpec::Start
        } else {
            AlignSpec::Center
        };
        layout.gap = Some(LengthSpec::Px(0.0));
        // Surface padding follows the spacing scale; a caller that declares its
        // own keeps it. A longhand outranks the `padding` shorthand when the
        // box resolves, so an edge the shorthand covers takes no default.
        let (horizontal, vertical) = if self.compact {
            (nana_ui_core::space::SM, nana_ui_core::space::MD)
        } else {
            (nana_ui_core::space::XXXL, nana_ui_core::space::PAGE)
        };
        crate::theme_intent::font(
            layout,
            if self.compact {
                nana_ui_core::TypeRole::Meta
            } else {
                nana_ui_core::TypeRole::Body
            },
        );
        crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Semibold);
        if layout.padding.is_none() && layout.padding_left.is_none() {
            layout.padding_left = Some(LengthSpec::Px(horizontal));
            layout.theme.padding_left = Some(crate::theme_intent::space(if self.compact {
                nana_ui_core::SpacingStep::Sm
            } else {
                nana_ui_core::SpacingStep::Xxxl
            }));
        }
        if layout.padding.is_none() && layout.padding_right.is_none() {
            layout.padding_right = Some(LengthSpec::Px(horizontal));
            layout.theme.padding_right = Some(crate::theme_intent::space(if self.compact {
                nana_ui_core::SpacingStep::Sm
            } else {
                nana_ui_core::SpacingStep::Xxxl
            }));
        }
        // The icon, title and message are laid out inside it, before the
        // action (#385), not written into it.
        if layout.padding.is_none() && layout.padding_top.is_none() {
            layout.padding_top = Some(LengthSpec::Px(vertical));
            layout.theme.padding_top = Some(crate::theme_intent::space(if self.compact {
                nana_ui_core::SpacingStep::Md
            } else {
                nana_ui_core::SpacingStep::Page
            }));
        }
        if layout.padding.is_none() && layout.padding_bottom.is_none() {
            layout.padding_bottom = Some(LengthSpec::Px(vertical));
            layout.theme.padding_bottom = Some(crate::theme_intent::space(if self.compact {
                nana_ui_core::SpacingStep::Md
            } else {
                nana_ui_core::SpacingStep::Page
            }));
        }
        layout.border_width = Some(0.0);
        style
    }
}

impl ComponentView for EmptyState {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "empty-state".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            "",
            self.effective_style(),
            StandardVisual::EmptyState {
                title: Arc::clone(&self.title),
                message: self.message.clone(),
                icon: self.icon,
                compact: self.compact,
                action: self.action,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.title)),
                value: self.message.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueEmphasis {
    Muted,
    Normal,
    #[default]
    Strong,
}

/// A compact label/value summary: label start-aligned, value end-aligned on one row.
/// An optional application-owned action child sits after the value.
/// The parent remains non-interactive; only the mounted child owns activation.
#[derive(Debug, Clone, PartialEq)]
pub struct LabeledValue {
    pub label: Arc<str>,
    pub value: Arc<str>,
    /// Treat the value as a read-only path, with middle elision and full tooltip.
    pub value_is_path: bool,
    pub emphasis: ValueEmphasis,
    pub compact: bool,
    /// An existing child view. No icon or action behavior is synthesized.
    pub action: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl LabeledValue {
    pub fn new(label: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            value_is_path: false,
            emphasis: ValueEmphasis::Strong,
            compact: true,
            action: None,
            style: NodeStyle::default(),
        }
    }

    pub fn path_value(mut self, path: impl Into<Arc<str>>) -> Self {
        self.value = path.into();
        self.value_is_path = true;
        self
    }

    pub fn emphasis(mut self, emphasis: ValueEmphasis) -> Self {
        self.emphasis = emphasis;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Declare an action child identity. Use
    /// [`crate::AppContext::set_labeled_value_action`] to validate and mount it.
    pub fn action_child(mut self, action: StableNodeId) -> Self {
        self.action = Some(action);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(FlexDirection::Row);
        layout.justify_content = if self.action.is_some() {
            JustifySpec::End
        } else {
            JustifySpec::SpaceBetween
        };
        layout.align_items = AlignSpec::Center;
        layout.min_height = Some(LengthSpec::Px(if self.compact { 28.0 } else { 32.0 }));
        layout.gap = Some(LengthSpec::Px(if self.compact { 4.0 } else { 8.0 }));
        style
    }
}

impl ComponentView for LabeledValue {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "labeled-value".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let value_role = match self.emphasis {
            ValueEmphasis::Muted => SemanticColorRole::Muted,
            ValueEmphasis::Normal | ValueEmphasis::Strong => SemanticColorRole::Text,
        };
        let value_weight = match self.emphasis {
            ValueEmphasis::Muted => 400,
            ValueEmphasis::Normal => 500,
            ValueEmphasis::Strong => 600,
        };
        project_visual(
            id,
            world,
            mutations,
            "",
            self.effective_style(),
            StandardVisual::LabeledValue {
                label: Arc::clone(&self.label),
                value: Arc::clone(&self.value),
                value_is_path: self.value_is_path,
                value_role,
                value_weight,
                compact: self.compact,
                action: self.action,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                value: Some(Arc::clone(&self.value)),
                ..AccessibilityState::default()
            },
        );
    }
}

const PROGRESS_GIRTH: f32 = nana_ui_core::space::SM;
const PROGRESS_LABEL_SIZE: f32 = nana_ui_core::type_scale::META;
const PROGRESS_GAP: f32 = nana_ui_core::space::SM;
const PROGRESS_CANCEL_SIZE: f32 = nana_ui_core::space::PAGE;
/// How much of its colour a still indeterminate fill keeps: faint enough
/// that a full track cannot be read as "done".
pub(crate) const PROGRESS_STILL_ALPHA: f32 = 0.35;
const SPINNER_DEFAULT_SIZE: f32 = nana_ui_core::space::XXL;
const SPINNER_LABEL_SIZE: f32 = nana_ui_core::type_scale::META;
const SPINNER_GAP: f32 = nana_ui_core::space::SM;

fn sanitize_progress_max(max: f64) -> f64 {
    if max.is_finite() && max > 0.0 {
        max
    } else {
        f64::MIN_POSITIVE
    }
}

fn sanitize_spinner_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        SPINNER_DEFAULT_SIZE
    }
}

/// Cancel request from a cancellable progress control. Progress does not own a timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressCancelled;

/// What a progress bar's fill shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressMode {
    /// `value / max` of the track, from its start.
    Value,
    /// The extent is unknown: a segment sweeps along the track, `phase` of
    /// the way through one pass (`0.0..=1.0`).
    Sweep { phase: f32 },
    /// The extent is unknown and motion is reduced: a faint fill across the
    /// whole track, which cannot be read as an amount.
    Still,
}

/// Progress track (Subtle rail, Accent fill, 6px girth): determinate, or
/// indeterminate while the task's extent is unknown.
///
/// Optional cancel is a real hit target.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub value: f64,
    pub max: f64,
    pub label: Option<Arc<str>>,
    pub cancellable: bool,
    /// The extent is unknown: a segment sweeps along the track instead of a
    /// fill standing for `value`, and assistive technology hears a busy bar
    /// with no amount. Under reduced motion the sweep holds still as a faint
    /// fill across the track.
    pub indeterminate: bool,
    /// The track's thickness; `None`, or one that is not a positive number,
    /// follows the installed theme's small spacing step.
    pub thickness: Option<f32>,
    /// The track's colour; `None` is Subtle.
    pub track: Option<SemanticColorRole>,
    /// The fill's colour; `None` is the theme's Indicator recipe.
    pub fill: Option<SemanticColorRole>,
    pub style: NodeStyle,
}

impl Progress {
    /// Playback longhands of the indeterminate sweep: pass after pass.
    pub(crate) const SWEEP_PLAYBACK: AnimationPlayback = AnimationPlayback {
        iteration_count: AnimationIteration::INFINITE,
        direction: AnimationDirection::Normal,
        fill_mode: AnimationFillMode::None,
        play_state: AnimationPlayState::Running,
        paused_at: None,
    };

    /// The sweep's timeline anchored at `start`, one pass every `pass`, or
    /// `None` when its hashed animation ID would be zero.
    pub(crate) fn sweep_animation(
        id: StableNodeId,
        start: Duration,
        pass: Duration,
    ) -> Option<AnimationSpec> {
        let animation = component_animation_id(component_animation_kinds::PROGRESS, id)?;
        Some(AnimationSpec::new(
            animation,
            id,
            start,
            pass,
            crate::framework::COMPONENT_FRAME_INTERVAL,
            Easing::Linear,
        ))
    }

    pub fn new(value: f64, max: f64) -> Self {
        Self {
            value,
            max: sanitize_progress_max(max),
            label: None,
            cancellable: false,
            indeterminate: false,
            thickness: None,
            track: None,
            fill: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }

    /// Whether the task's extent is unknown: a sweep, and a busy bar with no
    /// amount, instead of the value.
    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    /// The track's thickness.
    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    /// The track's colour.
    pub fn track(mut self, track: SemanticColorRole) -> Self {
        self.track = Some(track);
        self
    }

    /// The fill's colour.
    pub fn fill(mut self, fill: SemanticColorRole) -> Self {
        self.fill = Some(fill);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn resolved_max(&self) -> f64 {
        sanitize_progress_max(self.max)
    }

    fn explicit_thickness(&self) -> Option<f32> {
        self.thickness
            .filter(|thickness| thickness.is_finite() && *thickness > 0.0)
    }

    fn resolved_thickness(&self) -> f32 {
        self.explicit_thickness().unwrap_or(PROGRESS_GIRTH)
    }

    fn displayed_value(&self) -> f64 {
        let max = self.resolved_max();
        if self.value.is_finite() {
            self.value.clamp(0.0, max)
        } else {
            0.0
        }
    }

    fn value_ratio(&self) -> f32 {
        let ratio = (self.displayed_value() / self.resolved_max()) as f32;
        if ratio.is_finite() {
            ratio.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn heading_height(&self) -> f32 {
        if self.label.is_some() || self.cancellable {
            PROGRESS_LABEL_SIZE.max(if self.cancellable {
                PROGRESS_CANCEL_SIZE
            } else {
                0.0
            })
        } else {
            0.0
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(self.track.unwrap_or(SemanticColorRole::Subtle));
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        // A bar fills its row unless its style says how wide it is.
        layout.width.get_or_insert(LengthSpec::Fill);
        // Padding the caller declared is kept; only the unset edges default to
        // zero. Boxes are border-box, so the declared padding is added to the
        // content height instead of squeezing the bar.
        for edge in [
            &mut layout.padding_left,
            &mut layout.padding_right,
            &mut layout.padding_top,
            &mut layout.padding_bottom,
        ] {
            if edge.is_none() {
                *edge = Some(LengthSpec::Px(0.0));
            }
        }
        let padding = layout.resolved_padding();
        let heading = self.heading_height();
        let thickness = self.resolved_thickness();
        let content = if heading > 0.0 {
            heading + PROGRESS_GAP + thickness
        } else {
            thickness
        };
        layout.height = Some(LengthSpec::Px(content + padding.top + padding.bottom));
        layout.theme.height = self.explicit_thickness().is_none().then(|| {
            crate::theme_intent::space(nana_ui_core::SpacingStep::Sm)
                .offset(content - thickness + padding.top + padding.bottom)
        });
        layout.direction = Some(FlexDirection::Column);
        layout.gap = Some(LengthSpec::Px(PROGRESS_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
        layout.border_width = Some(0.0);
        if self.label.is_some() {
            layout.font_size = Some(PROGRESS_LABEL_SIZE);
            crate::theme_intent::font(layout, nana_ui_core::TypeRole::Meta);
            layout.font_weight = Some(nana_ui_core::type_scale::MEDIUM);
            crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Medium);
        }
        style
    }
}

impl ComponentView for Progress {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::cancel_progress),
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
            tag: "progress".into(),
        }
    }

    /// The sweep runs only while motion is not reduced: turning the
    /// preference either way starts or stops it, and changes what the fill
    /// shows.
    fn wants_reduced_motion_reproject() -> bool {
        true
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // An indeterminate bar sweeps on its own, as a spinner turns: start
        // the timeline once for a freshly created or mounted node and never
        // restart a running one, so repeated projections cannot reset it. A
        // hidden bar generates no box and stops it, as does a determinate one
        // or reduced motion.
        let sweeping = self.indeterminate && !world.reduced_motion();
        let pass = world
            .theme()
            .duration(nana_ui_core::MotionRole::SkeletonPulse);
        if let Some(spec) = Self::sweep_animation(id, Duration::ZERO, pass) {
            let running = world.animation_is_active(spec.id);
            if sweeping && !self.style.layout.hidden {
                let startable = !world.contains(id) || world.is_mounted(id);
                if startable && !running {
                    mutations.start_animation_with_playback(spec, Self::SWEEP_PLAYBACK);
                }
            } else if running {
                mutations.stop_animation(spec.id);
            }
        }
        let mode = if !self.indeterminate {
            ProgressMode::Value
        } else if !sweeping {
            ProgressMode::Still
        } else {
            // The timeline writes the phase straight into the visual; a
            // projection carries it on rather than restarting the pass.
            match world.standard_visual_ref(id) {
                Some(StandardVisual::Progress {
                    mode: ProgressMode::Sweep { phase },
                    ..
                }) => ProgressMode::Sweep { phase: *phase },
                _ => ProgressMode::Sweep { phase: 0.0 },
            }
        };
        let accessibility = if self.indeterminate {
            // No amount to report: a busy bar, as `<progress>` without a
            // value reads.
            AccessibilityState {
                role: AccessibilityRole::ProgressIndicator,
                label: self.label.clone(),
                busy: true,
                ..AccessibilityState::default()
            }
        } else {
            let percent = (f64::from(self.value_ratio()) * 100.0).round();
            AccessibilityState {
                role: AccessibilityRole::ProgressIndicator,
                label: self.label.clone(),
                value: Some(Arc::from(format!("{percent:.0}%"))),
                numeric_minimum: Some(0.0),
                numeric_maximum: Some(self.resolved_max()),
                numeric_value: Some(self.displayed_value()),
                ..AccessibilityState::default()
            }
        };
        project_visual(
            id,
            world,
            mutations,
            "",
            self.effective_style(),
            StandardVisual::Progress {
                value_ratio: self.value_ratio(),
                label: self.label.clone(),
                cancellable: self.cancellable,
                mode,
                thickness: self
                    .explicit_thickness()
                    .unwrap_or(world.theme().spacing().sm),
                fill: self.fill,
            },
            accessibility,
        );
        let interaction = InteractionState {
            pointer_events: self.cancellable,
            focusable: self.cancellable,
        };
        if world.interaction(id) != Some(interaction) {
            mutations.set_interaction(id, interaction);
        }
    }
}

/// Indeterminate loading indicator with an optional muted label.
///
/// Rotation is a Motion IR infinite timeline started on mount. The host does
/// not tick `phase`; samples write the visual. Hide or unmount to stop.
#[derive(Debug, Clone, PartialEq)]
pub struct Spinner {
    pub label: Arc<str>,
    /// Whether the label is painted beside the glyph. A hidden label is still
    /// the spinner's accessible name, for indicators that sit in tight rows.
    pub label_visible: bool,
    pub size: f32,
    /// Runtime-sampled rotation phase in `0.0..=1.0`. The animation dispatch
    /// owns this field, so a mounted spinner turns without the host ticking it.
    pub(crate) phase: f32,
    pub style: NodeStyle,
}

impl Spinner {
    /// Playback longhands of the infinite rotation timeline.
    pub(crate) const SPIN_PLAYBACK: AnimationPlayback = AnimationPlayback {
        iteration_count: AnimationIteration::INFINITE,
        direction: AnimationDirection::Normal,
        fill_mode: AnimationFillMode::None,
        play_state: AnimationPlayState::Running,
        paused_at: None,
    };

    /// The spinner's rotation timeline anchored at `start`, or `None` when its
    /// hashed animation ID would be zero.
    pub(crate) fn spin_animation(id: StableNodeId, start: Duration) -> Option<AnimationSpec> {
        let animation = component_animation_id(component_animation_kinds::SPINNER, id)?;
        Some(AnimationSpec::new(
            animation,
            id,
            start,
            nana_ui_core::motion::SPINNER_ROTATION,
            crate::framework::COMPONENT_FRAME_INTERVAL,
            Easing::Linear,
        ))
    }

    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            label_visible: true,
            size: SPINNER_DEFAULT_SIZE,
            phase: 0.0,
            style: NodeStyle::default(),
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = sanitize_spinner_size(size);
        self
    }

    /// Keep the label as the accessible name without painting it.
    pub fn label_visible(mut self, visible: bool) -> Self {
        self.label_visible = visible;
        self
    }

    fn painted_label(&self) -> &str {
        if self.label_visible {
            self.label.as_ref()
        } else {
            ""
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn resolved_size(&self) -> f32 {
        sanitize_spinner_size(self.size)
    }

    fn resolved_phase(&self) -> f32 {
        if self.phase.is_finite() {
            self.phase
        } else {
            0.0
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let size = self.resolved_size();
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Muted);
        style.background = None;
        style.border = None;
        style.text_vertical_alignment = TextVerticalAlignment::Center;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Shrink);
        layout.min_width = Some(LengthSpec::Px(size));
        layout.height = Some(LengthSpec::Px(size.max(SPINNER_DEFAULT_SIZE)));
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.gap = Some(LengthSpec::Px(SPINNER_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
        layout.padding_left = Some(LengthSpec::Px(if self.painted_label().is_empty() {
            0.0
        } else {
            size + SPINNER_GAP
        }));
        layout.theme.padding_left = if self.painted_label().is_empty() {
            None
        } else {
            Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm).offset(size))
        };
        // `padding_left` above is the spinner's own glyph lane and stays
        // framework-owned; the other three edges are the caller's.
        for edge in [
            &mut layout.padding_right,
            &mut layout.padding_top,
            &mut layout.padding_bottom,
        ] {
            if edge.is_none() {
                *edge = Some(LengthSpec::Px(0.0));
            }
        }
        layout.border_width = Some(0.0);
        layout.font_size = Some(SPINNER_LABEL_SIZE);
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Meta);
        style
    }
}

impl ComponentView for Spinner {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "spinner".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // A busy indicator turns on its own: start the timeline once for a
        // freshly created or mounted node and never restart a running one, so
        // repeated projections and per-frame phase writes cannot reset it.
        // A hidden spinner generates no box, so its timeline stops rather than
        // holding the host awake for a turn nobody can see.
        if let Some(spec) = Self::spin_animation(id, Duration::ZERO) {
            let startable = !world.contains(id) || world.is_mounted(id);
            let running = world.animation_is_active(spec.id);
            if self.style.layout.hidden {
                if running {
                    mutations.stop_animation(spec.id);
                }
            } else if startable && !running {
                mutations.start_animation_with_playback(spec, Self::SPIN_PLAYBACK);
            }
        }
        project_visual(
            id,
            world,
            mutations,
            self.painted_label(),
            self.effective_style(),
            StandardVisual::Spinner {
                label: Arc::from(self.painted_label()),
                size: self.resolved_size(),
                phase: self.resolved_phase(),
            },
            AccessibilityState {
                role: AccessibilityRole::ProgressIndicator,
                label: Some(Arc::clone(&self.label)),
                busy: true,
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppContext, Button, DocumentId, FrameworkError, ProgressCancelled, StandardVisual,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use unicode_segmentation::UnicodeSegmentation;

    #[derive(Default)]
    struct WrappingShaper;

    impl crate::TextShaper for WrappingShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &crate::TextContent,
            style: &crate::ComputedStyle,
            constraints: crate::TextShapeConstraints,
        ) -> crate::TextMetrics {
            let count = text.value.graphemes(true).count();
            if count == 0 {
                return crate::TextMetrics::default();
            }
            let advance = style.font_size;
            let natural_width = count as f32 * advance;
            let columns = constraints
                .max_width
                .filter(|_| constraints.wrap)
                .map(|width| (width / advance).floor().max(1.0) as usize)
                .unwrap_or(count);
            let lines = count.div_ceil(columns);
            crate::TextMetrics {
                width: constraints
                    .max_width
                    .map_or(natural_width, |width| natural_width.min(width)),
                height: lines as f32 * style.font_size * 1.2,
                ascent: None,
            }
        }
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn layout(context: &mut AppContext, id: StableNodeId, width: f32, height: f32) {
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            id,
            crate::LayoutBox {
                x: 10.0,
                y: 20.0,
                width,
                height,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
    }

    #[test]
    fn hidden_spinner_label_names_it_without_painting_text() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let spinner = context
            .create_component(document, Spinner::new("处理中").label_visible(false))
            .unwrap();
        let id = spinner.stable_id();
        assert_eq!(context.world().text(id), Some(""));
        assert_eq!(
            context
                .world()
                .accessibility(id)
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("处理中")
        );
    }

    #[test]
    fn status_badge_projects_tone_density_and_accessible_label() {
        let mut context = AppContext::new();
        let badge = context
            .create_component(
                document(),
                StatusBadge::new("Offline", nana_ui_core::StatusTone::Danger).compact(false),
            )
            .unwrap();
        let id = badge.stable_id();

        assert_eq!(context.world().text(id), Some("Offline"));
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Danger)
        );
        assert_eq!(
            context.world().node_style(id).unwrap().layout.font_size,
            Some(nana_ui_core::type_scale::META)
        );
        assert_eq!(
            context.world().accessibility(id).unwrap().label.as_deref(),
            Some("Offline")
        );
        layout(&mut context, id, 96.0, 24.0);
        let crate::ComponentGeometry::StatusBadge {
            indicator,
            label,
            background,
            foreground,
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("status badge geometry")
        };
        assert!(indicator.width > 0.0);
        assert_eq!(label.font_size, nana_ui_core::type_scale::META);
        assert_eq!(label.font_weight, Some(nana_ui_core::type_scale::MEDIUM));
        assert_eq!(background[..3], foreground[..3]);
        assert!(background[3] < foreground[3]);
        let dark_foreground = foreground;
        let mut theme = MutationQueue::new();
        theme.set_preset_theme(crate::ThemeAppearance::Light);
        context.commit_mutations(theme).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let crate::ComponentGeometry::StatusBadge { foreground, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("status badge geometry")
        };
        assert_ne!(foreground, dark_foreground);
    }

    #[test]
    fn validation_message_projects_intent_and_invalid_semantics() {
        let mut context = AppContext::new();
        let message = context
            .create_component(
                document(),
                ValidationMessage::new(
                    "A project is required",
                    nana_ui_core::ValidationIntent::Warning,
                ),
            )
            .unwrap();
        let id = message.stable_id();

        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Warning)
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert!(accessibility.invalid);
        assert_eq!(
            accessibility.label.as_deref(),
            Some("A project is required")
        );
        layout(&mut context, id, 180.0, 18.0);
        let crate::ComponentGeometry::ValidationMessage {
            indicator,
            label,
            foreground,
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("validation geometry")
        };
        assert!(indicator.width > 0.0);
        assert_eq!(label.font_size, nana_ui_core::type_scale::HINT);
        assert_eq!(label.color, Some(foreground));
        // The geometry carries the resolved theme weight; the authored style
        // stays unset so a parent or a live theme can supply it.
        assert_eq!(
            label.font_weight,
            Some(context.world().theme().typography().regular)
        );
        assert_eq!(
            context.world().node_style(id).unwrap().layout.font_weight,
            None
        );
    }

    #[test]
    fn empty_state_wrapped_intrinsics_drive_action_layout_and_settle() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("继续"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("没有可显示的项目👩‍💻请稍后再试")
                    .message("较长的说明文字会根据最终内容宽度自动换行🙂")
                    .icon(Icon::Folder),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 400.0))
            .unwrap();
        let first_action_y = context
            .world()
            .component_layout_box(action.stable_id())
            .unwrap()
            .y;

        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 400.0))
            .unwrap();
        assert!(
            !context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );

        let crate::ComponentGeometry::EmptyState {
            title,
            message: Some(message),
            action: Some(action_bounds),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("wrapped empty state geometry")
        };
        assert!(title.bounds.height > 13.0 * 1.2);
        assert!(message.bounds.height > 12.0 * 1.2);
        assert!(action_bounds.y > first_action_y);
        assert!((action_bounds.y - (message.bounds.y + message.bounds.height) - 10.0).abs() < 0.01);

        let metrics = (title.bounds.height, message.bounds.height, action_bounds.y);
        context
            .set_preset_theme(crate::ThemeAppearance::Light)
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        assert!(
            !context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        let crate::ComponentGeometry::EmptyState {
            title,
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("light empty state geometry")
        };
        assert_eq!(
            metrics,
            (title.bounds.height, message.bounds.height, action.y)
        );
    }

    #[test]
    fn empty_state_title_only_has_no_synthetic_spacing() {
        let mut context = AppContext::new();
        let empty = context
            .create_component(document(), EmptyState::new("空").compact(true))
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(100.0, 100.0))
            .unwrap();
        context
            .shape_text_for_layout(document(), &mut shaper)
            .unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(100.0, 100.0))
            .unwrap();
        let crate::ComponentGeometry::EmptyState {
            icon: None,
            message: None,
            action: None,
            title,
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("title-only empty state geometry")
        };
        assert_eq!(title.bounds.height, 12.0 * 1.2);
        // The title is laid out inside the padding, before the children
        // (#385): the padding is the compact default alone.
        assert_eq!(
            context
                .world()
                .node_style(empty.stable_id())
                .unwrap()
                .layout
                .padding_top,
            Some(LengthSpec::Px(8.0))
        );
        assert_eq!(
            context
                .world()
                .leading_parts(empty.stable_id())
                .map(|lead| lead.extent),
            Some(title.bounds.height)
        );
    }

    #[test]
    fn normal_empty_state_centers_shaped_regions_while_preserving_start_aligned_lines() {
        let mut context = AppContext::new();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("ABCDE").message("这是用于验证窄宽换行起点的较长消息文本"),
            )
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 200.0))
            .unwrap();
        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 200.0))
            .unwrap();

        let crate::ComponentGeometry::EmptyState {
            content_clip: clip,
            title,
            message: Some(message),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("normal empty state geometry")
        };
        assert!(message.bounds.height > 12.0 * 1.2);
        assert!(title.bounds.width < message.bounds.width);
        assert!(title.bounds.x > message.bounds.x);
        assert!((title.bounds.x - (clip.x + (clip.width - title.bounds.width) / 2.0)).abs() < 0.01);
        assert!(
            (message.bounds.x - (clip.x + (clip.width - message.bounds.width) / 2.0)).abs() < 0.01
        );
    }

    #[test]
    fn extremely_narrow_empty_state_keeps_intrinsics_ordered_inside_content_width() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("标题")
                    .icon(Icon::Folder)
                    .message("包含 emoji 🎉 的窄宽消息"),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(40.0, 400.0))
            .unwrap();
        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(40.0, 400.0))
            .unwrap();

        let crate::ComponentGeometry::EmptyState {
            content_clip: clip,
            icon: Some((_, icon, _)),
            title,
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("narrow empty state geometry")
        };
        for bounds in [icon, title.bounds, message.bounds] {
            assert!(bounds.x >= clip.x);
            assert!(bounds.x + bounds.width <= clip.x + clip.width + 0.01);
        }
        assert!(icon.y + icon.height <= title.bounds.y + 0.01);
        assert!(title.bounds.y + title.bounds.height <= message.bounds.y + 0.01);
        assert!(message.bounds.y + message.bounds.height <= action.y + 0.01);
    }

    #[test]
    fn empty_state_root_clip_bounds_action_hit_testing_and_accessibility() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(document(), EmptyState::new("Empty"))
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let mut layouts = MutationQueue::new();
        layouts.write_layout(
            empty.stable_id(),
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        layouts.write_layout(
            action.stable_id(),
            crate::LayoutBox {
                x: 80.0,
                y: 40.0,
                width: 40.0,
                height: 20.0,
            },
        );
        context.commit_mutations(layouts).unwrap();
        context.compat_world_mut().rebuild_hit_test(document());

        assert_eq!(
            context.world().hit_test(document(), 95.0, 45.0),
            Some(action.stable_id())
        );
        assert_ne!(
            context.world().hit_test(document(), 110.0, 45.0),
            Some(action.stable_id())
        );
        let action_accessibility = context
            .world()
            .project_accessibility(document())
            .into_iter()
            .find(|node| node.id == action.stable_id())
            .unwrap();
        assert_eq!(
            action_accessibility.bounds,
            crate::LayoutBox {
                x: 80.0,
                y: 40.0,
                width: 20.0,
                height: 10.0,
            }
        );

        let mut outside = MutationQueue::new();
        outside.write_layout(
            action.stable_id(),
            crate::LayoutBox {
                x: 110.0,
                y: 40.0,
                width: 20.0,
                height: 10.0,
            },
        );
        context.commit_mutations(outside).unwrap();
        context.compat_world_mut().rebuild_hit_test(document());
        assert_ne!(
            context.world().hit_test(document(), 115.0, 45.0),
            Some(action.stable_id())
        );
        let accessibility = context.world().project_accessibility(document());
        assert!(
            accessibility
                .iter()
                .all(|node| node.id != action.stable_id())
        );
        assert!(
            accessibility
                .iter()
                .find(|node| node.id == empty.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
    }

    #[test]
    fn compact_empty_state_action_gap_combines_column_and_action_padding() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("Title").message("Message").compact(true),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(160.0, 120.0))
            .unwrap();
        context
            .shape_text_for_layout(document(), &mut shaper)
            .unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(160.0, 120.0))
            .unwrap();
        let crate::ComponentGeometry::EmptyState {
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("compact action geometry")
        };
        assert!((action.y - (message.bounds.y + message.bounds.height) - 6.0).abs() < 0.01);
    }

    #[test]
    fn empty_state_owns_semantic_content_and_action_mount_is_atomic() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Create project").loading(true))
            .unwrap();
        let replacement = context
            .create_detached_component(document(), Button::new("Import project"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("No projects")
                    .icon(Icon::Folder)
                    .message("Create the first project"),
            )
            .unwrap();
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert!(
            !context
                .world()
                .document_order(document())
                .contains(&action.stable_id())
        );
        assert_eq!(context.next_animation_deadline(), None);
        assert_eq!(
            context.world().interaction(empty.stable_id()),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let Some(StandardVisual::EmptyState {
            icon,
            message,
            action: None,
            ..
        }) = context.world().standard_visual(empty.stable_id())
        else {
            panic!("empty state visual");
        };
        assert_eq!(icon, Some(Icon::Folder));
        assert_eq!(message.as_deref(), Some("Create the first project"));
        assert!(
            context
                .set_empty_state_action(empty, Some(action.stable_id()))
                .unwrap()
        );
        assert!(
            !context
                .set_empty_state_action(empty, Some(action.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![action.stable_id()]
        );
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

        let missing = StableNodeId::new(999).unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, Some(missing)),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == empty.stable_id() && slot == missing
        ));
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![action.stable_id()]
        );
        assert!(
            context
                .set_empty_state_action(empty, Some(replacement.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![replacement.stable_id()]
        );
        assert!(
            context
                .world()
                .node(action.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert_eq!(context.next_animation_deadline(), None);
        assert!(context.set_empty_state_action(empty, None).unwrap());
        assert!(
            context
                .world()
                .node(empty.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
        assert!(
            context
                .world()
                .node(replacement.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(replacement.stable_id()),
            Some(crate::MountState::Parked)
        );
        layout(&mut context, empty.stable_id(), 240.0, 120.0);
        let crate::ComponentGeometry::EmptyState {
            icon,
            title,
            message,
            action,
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("empty state geometry")
        };
        let Some((kind, _, _)) = icon else {
            panic!("empty state icon");
        };
        assert_eq!(kind, Icon::Folder);
        assert_eq!(title.font_size, 13.0);
        assert_eq!(message.unwrap().font_size, 12.0);
        assert!(action.is_none());
    }

    #[test]
    fn labeled_value_action_is_validated_and_remains_application_owned() {
        let mut context = AppContext::new();
        let foreign_action = context
            .create_detached_component(DocumentId::new(2).unwrap(), Button::new("Foreign"))
            .unwrap();
        let action = context
            .create_detached_component(document(), Button::new("Copy"))
            .unwrap();
        let replacement = context
            .create_detached_component(document(), Button::new("Reveal"))
            .unwrap();
        let owner = context
            .create_component(document(), Button::new("Owner"))
            .unwrap();
        let owned_action = context
            .create_detached_component(document(), Button::new("Owned"))
            .unwrap();
        context.append_child(owner, owned_action).unwrap();
        let summary = context
            .create_component(
                document(),
                LabeledValue::new("Revision", "42").emphasis(ValueEmphasis::Normal),
            )
            .unwrap();
        let id = summary.stable_id();

        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let missing = StableNodeId::new(999).unwrap();
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(missing)),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == missing
        ));
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(foreign_action.stable_id())),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == foreign_action.stable_id()
        ));
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(owned_action.stable_id())),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == owned_action.stable_id()
        ));
        assert_eq!(context.read(summary, |view| view.action).unwrap(), None);
        assert!(context.world().node(id).unwrap().children.is_empty());
        assert_eq!(
            context
                .world()
                .node(owned_action.stable_id())
                .unwrap()
                .parent,
            Some(owner.stable_id())
        );

        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(action, move |_button, _event: &crate::Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        assert!(
            context
                .set_labeled_value_action(summary, Some(action.stable_id()))
                .unwrap()
        );

        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::LabeledValue {
                label: Arc::from("Revision"),
                value: Arc::from("42"),
                value_is_path: false,
                value_role: SemanticColorRole::Text,
                value_weight: 500,
                compact: true,
                action: Some(action.stable_id()),
            })
        );
        assert_eq!(context.world().text(id), Some(""));
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert_eq!(
            context.world().node(id).unwrap().children,
            vec![action.stable_id()]
        );
        assert!(context.activate_button(action).unwrap());
        assert_eq!(*activations.lock().unwrap(), 1);
        assert!(
            context
                .set_labeled_value_action(summary, Some(replacement.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(id).unwrap().children,
            vec![replacement.stable_id()]
        );
        assert!(
            context
                .world()
                .node(action.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert!(
            context
                .set_labeled_value_action(summary, Some(action.stable_id()))
                .unwrap()
        );
        assert!(context.activate_button(action).unwrap());
        assert_eq!(*activations.lock().unwrap(), 2);
        assert!(context.set_labeled_value_action(summary, None).unwrap());
        assert!(context.world().node(id).unwrap().children.is_empty());
        let removed = context.remove_view(action).unwrap();
        assert_eq!(removed.label, "Copy");
        assert!(!context.world().contains(action.stable_id()));
        layout(&mut context, id, 180.0, 32.0);
        let crate::ComponentGeometry::LabeledValue { label, value, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("labeled value geometry")
        };
        assert_eq!(label.font_size, nana_ui_core::type_scale::HINT);
        assert_eq!(value.font_size, nana_ui_core::type_scale::META);
        assert_eq!(value.font_weight, Some(nana_ui_core::type_scale::MEDIUM));
        assert_ne!(label.color, value.color);
        assert!(
            label.bounds.x + label.bounds.width <= value.bounds.x + 0.01,
            "label must sit left of value, got label={:?} value={:?}",
            label.bounds,
            value.bounds
        );
        assert!(
            label.bounds.y < value.bounds.y + value.bounds.height
                && value.bounds.y < label.bounds.y + label.bounds.height,
            "label and value must share one row, got label={:?} value={:?}",
            label.bounds,
            value.bounds
        );
        assert!(
            (value.bounds.x + value.bounds.width - 190.0).abs() < 1.0,
            "value must hug the trailing edge, got {:?}",
            value.bounds
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("Revision"));
        assert_eq!(accessibility.value.as_deref(), Some("42"));
    }

    /// 与 world.rs `estimated_text_width` 相同的字符分类估算(ASCII≈0.62×字号,其余≈1.0×字号),
    /// world 模块私有,测试侧镜像一份用于推导自然宽度契约。
    fn estimated_width(text: &str, font_size: f32) -> f32 {
        text.chars()
            .map(|ch| {
                if ch.is_ascii() {
                    font_size * 0.62
                } else {
                    font_size
                }
            })
            .sum::<f32>()
            .max(font_size)
    }

    #[test]
    fn labeled_value_geometry_keeps_label_natural_width_under_long_value() {
        let mut context = AppContext::new();
        let summary = context
            .create_component(
                document(),
                LabeledValue::new(
                    "路径",
                    "/Users/someone/Library/Application Support/models/xxx/model.model3.json",
                ),
            )
            .unwrap();
        let id = summary.stable_id();
        layout(&mut context, id, 180.0, 32.0);
        let crate::ComponentGeometry::LabeledValue { label, value, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("labeled value geometry")
        };
        let label_natural = estimated_width("路径", label.font_size);
        assert!(
            label.bounds.width + 0.01 >= label_natural,
            "label must keep its natural width, got {:?} (natural {label_natural})",
            label.bounds
        );
        assert!(
            (value.bounds.width - (180.0 - label_natural - 4.0)).abs() < 1.0,
            "overflowing value must take the remaining width so only the excess becomes an \
             ellipsis, got {:?}",
            value.bounds
        );
        assert!(
            (value.bounds.x + value.bounds.width - 190.0).abs() < 1.0,
            "value must hug the trailing edge, got {:?}",
            value.bounds
        );
        assert!(
            label.bounds.x + label.bounds.width <= value.bounds.x + 0.01,
            "label must not overlap value, got label={:?} value={:?}",
            label.bounds,
            value.bounds
        );
    }

    #[test]
    fn labeled_value_geometry_keeps_regions_disjoint_when_both_sides_overflow() {
        let mut context = AppContext::new();
        let summary = context
            .create_component(
                document(),
                LabeledValue::new(
                    "网络请求失败后的重试策略与指数退避间隔说明",
                    "https://example.com/very/long/path/that/overflows/the/row/index.html",
                ),
            )
            .unwrap();
        let id = summary.stable_id();
        layout(&mut context, id, 180.0, 32.0);
        let crate::ComponentGeometry::LabeledValue { label, value, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("labeled value geometry")
        };
        assert!(
            label.bounds.width >= 0.0 && value.bounds.width >= 0.0,
            "region widths must be non-negative, got label={:?} value={:?}",
            label.bounds,
            value.bounds
        );
        assert!(
            label.bounds.x + label.bounds.width <= value.bounds.x + 0.01,
            "regions must not overlap, got label={:?} value={:?}",
            label.bounds,
            value.bounds
        );
        assert!(
            value.bounds.width + 0.01 >= estimated_width("xx", value.font_size),
            "value must keep a minimal visible width, got {:?}",
            value.bounds
        );
        assert!(
            (value.bounds.x + value.bounds.width - 190.0).abs() < 1.0,
            "value must hug the trailing edge, got {:?}",
            value.bounds
        );
    }

    #[test]
    fn feedback_helpers_never_park_unowned_field_only_actions() {
        let mut context = AppContext::new();
        let foreign = context
            .create_detached_component(DocumentId::new(2).unwrap(), Button::new("Foreign"))
            .unwrap();
        let empty = context
            .create_component(document(), EmptyState::new("Empty"))
            .unwrap();
        context
            .update_component(empty, |empty, _cx| {
                empty.action = Some(foreign.stable_id());
            })
            .unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot),
            }) if parent == empty.stable_id() && slot == foreign.stable_id()
        ));
        assert_eq!(
            context.read(empty, |empty| empty.action).unwrap(),
            Some(foreign.stable_id())
        );
        assert_eq!(
            context.world().mount_state(foreign.stable_id()),
            Some(crate::MountState::Parked)
        );

        let missing = StableNodeId::new(999).unwrap();
        context
            .update_component(empty, |empty, _cx| empty.action = Some(missing))
            .unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                slot: Some(slot),
                ..
            }) if slot == missing
        ));

        let stale = context
            .create_detached_component(document(), Button::new("Stale"))
            .unwrap();
        context
            .update_component(empty, |empty, _cx| {
                empty.action = Some(stale.stable_id());
            })
            .unwrap();
        assert!(context.set_empty_state_action(empty, None).is_err());
        assert_eq!(
            context.world().mount_state(stale.stable_id()),
            Some(crate::MountState::Parked)
        );

        let declared = context
            .create_detached_component(document(), Button::new("Declared"))
            .unwrap();
        let declared_empty = context
            .create_component(
                document(),
                EmptyState::new("Declared empty").action_child(declared.stable_id()),
            )
            .unwrap();
        assert!(
            context
                .set_empty_state_action(declared_empty, Some(declared.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(declared.stable_id()).unwrap().parent,
            Some(declared_empty.stable_id())
        );

        let owner = context
            .create_component(document(), Button::new("Owner"))
            .unwrap();
        let owned = context
            .create_detached_component(document(), Button::new("Owned"))
            .unwrap();
        context.append_child(owner, owned).unwrap();
        let summary = context
            .create_component(document(), LabeledValue::new("Key", "Value"))
            .unwrap();
        context
            .update_component(summary, |summary, _cx| {
                summary.action = Some(owned.stable_id());
            })
            .unwrap();
        assert!(matches!(
            context.set_labeled_value_action(summary, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot),
            }) if parent == summary.stable_id() && slot == owned.stable_id()
        ));
        assert_eq!(
            context.world().node(owned.stable_id()).unwrap().parent,
            Some(owner.stable_id())
        );
        assert_eq!(
            context.world().mount_state(owned.stable_id()),
            Some(crate::MountState::Mounted)
        );
        assert!(
            context
                .world()
                .node(summary.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
    }

    fn progress_ratio(context: &AppContext, id: StableNodeId) -> f32 {
        match context.world().standard_visual(id) {
            Some(StandardVisual::Progress { value_ratio, .. }) => value_ratio,
            other => panic!("progress visual: {other:?}"),
        }
    }

    fn progress_fill_width(context: &AppContext, id: StableNodeId) -> (f32, f32) {
        let crate::ComponentGeometry::Progress { track, fill, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("progress geometry")
        };
        (track.width, fill.width)
    }

    #[test]
    fn progress_clamps_value_and_fill_width() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(document(), Progress::new(-5.0, 100.0))
            .unwrap();
        let id = progress.stable_id();
        assert_eq!(progress_ratio(&context, id), 0.0);
        assert!(Progress::new(1.0, 0.0).max > 0.0);
        assert!(Progress::new(1.0, f64::NAN).max.is_finite());
        assert!(Progress::new(1.0, f64::INFINITY).max.is_finite());

        context
            .update_component(progress, |progress, _| {
                progress.value = 50.0;
            })
            .unwrap();
        assert_eq!(progress_ratio(&context, id), 0.5);
        layout(&mut context, id, 100.0, 6.0);
        let (track, fill) = progress_fill_width(&context, id);
        assert_eq!(track, 100.0);
        assert!((fill - 50.0).abs() < 0.01);

        context
            .update_component(progress, |progress, _| {
                progress.value = 0.0;
            })
            .unwrap();
        layout(&mut context, id, 100.0, 6.0);
        let (_, fill) = progress_fill_width(&context, id);
        assert_eq!(fill, 0.0);

        context
            .update_component(progress, |progress, _| {
                progress.value = 125.0;
            })
            .unwrap();
        assert_eq!(progress_ratio(&context, id), 1.0);
        layout(&mut context, id, 100.0, 6.0);
        let (track, fill) = progress_fill_width(&context, id);
        assert_eq!(fill, track);
    }

    #[test]
    fn progress_cancellable_reserves_cancel_hit_target() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(
                document(),
                Progress::new(40.0, 100.0)
                    .label("Copying")
                    .cancellable(true),
            )
            .unwrap();
        let id = progress.stable_id();
        layout(&mut context, id, 160.0, 36.0);
        let crate::ComponentGeometry::Progress {
            cancel: Some(cancel),
            label: Some(label),
            ..
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("cancellable progress geometry");
        };
        assert!(cancel.width > 0.0);
        assert!(label.bounds.width < 160.0);
        assert!(context.world().interaction(id).unwrap().pointer_events);

        let cancelled = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&cancelled);
        context
            .on(
                progress,
                move |_progress, _event: &ProgressCancelled, _cx| {
                    *flag.lock().unwrap() = true;
                },
            )
            .unwrap();
        assert!(context.cancel_progress(progress).unwrap());
        assert!(*cancelled.lock().unwrap());
    }

    #[test]
    fn progress_idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(document(), Progress::new(40.0, 100.0).label("Copying"))
            .unwrap();
        let _ = context.take_system_work();
        context.reproject_component(progress).unwrap();
        assert!(context.take_system_work().is_empty());
        let accessibility = context.world().accessibility(progress.stable_id()).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::ProgressIndicator);
        assert_eq!(accessibility.value.as_deref(), Some("40%"));
        assert_eq!(accessibility.label.as_deref(), Some("Copying"));
    }

    /// A bar fills its row; a style that says how wide it is wins, so a bar
    /// stretched across a column does not widen what sizes to its content.
    #[test]
    fn a_progress_bar_fills_its_row_unless_its_style_gives_a_width() {
        let mut context = AppContext::new();
        let width = |context: &AppContext, id: StableNodeId| {
            context.world().node_style(id).unwrap().layout.width
        };
        let filling = context
            .create_component(document(), Progress::new(1.0, 2.0))
            .unwrap();
        assert_eq!(width(&context, filling.stable_id()), Some(LengthSpec::Fill));
        let mut sized = Progress::new(1.0, 2.0);
        Arc::make_mut(&mut sized.style.layout).width = Some(LengthSpec::Px(120.0));
        let sized = context.create_component(document(), sized).unwrap();
        assert_eq!(
            width(&context, sized.stable_id()),
            Some(LengthSpec::Px(120.0))
        );
    }

    #[test]
    fn spinner_projects_visual_and_label() {
        let mut context = AppContext::new();
        let spinner = context
            .create_component(document(), Spinner::new("Loading"))
            .unwrap();
        let id = spinner.stable_id();
        assert_eq!(context.world().text(id), Some("Loading"));
        match context.world().standard_visual(id) {
            Some(StandardVisual::Spinner { label, size, phase }) => {
                assert_eq!(&*label, "Loading");
                assert_eq!(size, 14.0);
                assert_eq!(phase, 0.0);
            }
            other => panic!("spinner visual: {other:?}"),
        }
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::ProgressIndicator);
        assert!(accessibility.busy);
        assert_eq!(accessibility.label.as_deref(), Some("Loading"));
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Muted)
        );
        assert_eq!(
            context.world().node_style(id).unwrap().layout.height,
            Some(LengthSpec::Px(14.0))
        );
    }

    #[test]
    fn a_mounted_spinner_turns_without_the_host_ticking_it() {
        fn projected_phase(context: &AppContext, id: StableNodeId) -> f32 {
            match context.world().standard_visual(id) {
                Some(StandardVisual::Spinner { phase, .. }) => phase,
                other => panic!("expected a spinner visual, got {other:?}"),
            }
        }

        let mut context = AppContext::new();
        let spinner = context
            .create_component(document(), Spinner::new("Loading"))
            .unwrap();
        let id = spinner.stable_id();
        let _ = context.take_system_work();

        let steady = context.advance_animations(Duration::ZERO);
        assert!(steady.has_updates());
        assert_eq!(projected_phase(&context, id), 0.0);
        assert!(
            steady
                .next_deadline
                .is_some_and(|deadline| deadline > Duration::ZERO),
            "the timeline schedules its own follow-up frames"
        );

        context.advance_animations(Duration::from_millis(450));
        let half = projected_phase(&context, id);
        assert!((half - 0.5).abs() < 1e-4, "half a turn at half the period");
        assert!(!context.take_system_work().is_empty());

        context.advance_animations(Duration::from_millis(675));
        assert!(projected_phase(&context, id) > half, "the turn continues");
        assert!(
            context.next_animation_deadline().is_some(),
            "an indeterminate spinner never settles"
        );

        // A hidden spinner must not hold the host awake for an invisible turn.
        context
            .update_component(spinner, |spinner, _| {
                Arc::make_mut(&mut spinner.style.layout).hidden = true;
            })
            .unwrap();
        assert_eq!(context.next_animation_deadline(), None);
        context
            .update_component(spinner, |spinner, _| {
                Arc::make_mut(&mut spinner.style.layout).hidden = false;
            })
            .unwrap();
        assert!(
            context.next_animation_deadline().is_some(),
            "showing it again resumes the turn"
        );
    }

    fn progress_boxes(
        context: &AppContext,
        id: StableNodeId,
    ) -> (crate::LayoutBox, crate::LayoutBox) {
        let crate::ComponentGeometry::Progress { track, fill, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("progress geometry")
        };
        (track, fill)
    }

    fn progress_mode(context: &AppContext, id: StableNodeId) -> ProgressMode {
        match context.world().standard_visual(id) {
            Some(StandardVisual::Progress { mode, .. }) => mode,
            other => panic!("progress visual: {other:?}"),
        }
    }

    /// A bar may name its own rail: the track is the node's surface, the
    /// fill the visual's colour, and the box is as tall as the thickness.
    /// Without them it is the bar it always was.
    #[test]
    fn a_progress_bar_takes_its_own_thickness_track_and_fill() {
        let palette = nana_ui_core::SemanticPalette::dark();
        let mut context = AppContext::new();
        let progress = context
            .create_component(
                document(),
                Progress::new(25.0, 100.0)
                    .thickness(3.0)
                    .track(SemanticColorRole::BorderSoft)
                    .fill(SemanticColorRole::Warning),
            )
            .unwrap();
        let id = progress.stable_id();
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::BorderSoft));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(3.0)));
        layout(&mut context, id, 100.0, 3.0);
        let crate::ComponentGeometry::Progress {
            track,
            fill,
            corner_radius,
            ..
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("progress geometry")
        };
        assert_eq!(track.height, 3.0);
        assert_eq!(corner_radius, 1.5, "still a pill");
        assert!((fill.width - 25.0).abs() < 0.01);
        assert_eq!(
            context.world().extract_nodes(&[id])[0].standard_visual_foreground,
            Some(palette.warning.as_rgba_array())
        );

        context
            .update_component(progress, |progress, _| {
                progress.thickness = Some(f32::NAN);
            })
            .unwrap();
        assert_eq!(
            context.world().node_style(id).unwrap().layout.height,
            Some(LengthSpec::Px(PROGRESS_GIRTH)),
            "a thickness that is not a positive number keeps the rail"
        );

        let plain = context
            .create_component(document(), Progress::new(25.0, 100.0))
            .unwrap();
        let style = context.world().node_style(plain.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Subtle));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(6.0)));
        assert_eq!(
            context.world().extract_nodes(&[plain.stable_id()])[0].standard_visual_foreground,
            Some(palette.accent.as_rgba_array())
        );
    }

    /// An indeterminate bar has no amount to report: assistive technology
    /// hears a busy progress indicator with no value. Made determinate, it
    /// reports its percentage again.
    #[test]
    fn an_indeterminate_bar_is_busy_and_reports_no_amount() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(
                document(),
                Progress::new(40.0, 100.0)
                    .label("Exporting")
                    .indeterminate(true),
            )
            .unwrap();
        let id = progress.stable_id();
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::ProgressIndicator);
        assert!(accessibility.busy);
        assert_eq!(accessibility.label.as_deref(), Some("Exporting"));
        assert_eq!(accessibility.value, None);
        assert_eq!(
            (
                accessibility.numeric_minimum,
                accessibility.numeric_maximum,
                accessibility.numeric_value
            ),
            (None, None, None)
        );

        context
            .update_component(progress, |progress, _| progress.indeterminate = false)
            .unwrap();
        let accessibility = context.world().accessibility(id).unwrap();
        assert!(!accessibility.busy);
        assert_eq!(accessibility.value.as_deref(), Some("40%"));
        assert_eq!(accessibility.numeric_maximum, Some(100.0));
        assert_eq!(accessibility.numeric_value, Some(40.0));
    }

    /// The sweep is a loop the bar starts itself: its segment comes in at
    /// the start of the track and goes out past the end over one pass, and
    /// a projection carries the pass on. Reduced motion stops the loop and
    /// holds a faint fill across the whole track; letting motion back starts
    /// it again, and a determinate bar stops it for good.
    #[test]
    fn an_indeterminate_bar_sweeps_until_motion_is_reduced() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(document(), Progress::new(0.0, 1.0).indeterminate(true))
            .unwrap();
        let id = progress.stable_id();
        let pass = context
            .world()
            .theme()
            .duration(nana_ui_core::MotionRole::SkeletonPulse);
        let sweep = Progress::sweep_animation(id, Duration::ZERO, pass)
            .unwrap()
            .id;
        assert!(context.world().animation_is_active(sweep));
        assert_eq!(
            progress_mode(&context, id),
            ProgressMode::Sweep { phase: 0.0 }
        );

        context.advance_animations(Duration::ZERO);
        context.advance_animations(pass / 2);
        let ProgressMode::Sweep { phase } = progress_mode(&context, id) else {
            panic!("a sweeping bar");
        };
        assert!((phase - 0.5).abs() < 1e-3, "half a pass at half the period");
        layout(&mut context, id, 200.0, 6.0);
        let (track, fill) = progress_boxes(&context, id);
        assert!(
            (fill.width - 70.0).abs() < 0.01 && (fill.x - (track.x + 65.0)).abs() < 0.01,
            "half way, a segment 35% of the track sits in its middle: {fill:?}"
        );
        context.reproject_component(progress).unwrap();
        assert_eq!(progress_mode(&context, id), ProgressMode::Sweep { phase });
        assert!(
            context.next_animation_deadline().is_some(),
            "an indeterminate bar never settles"
        );

        context.set_reduced_motion(true);
        assert!(!context.world().animation_is_active(sweep));
        assert_eq!(progress_mode(&context, id), ProgressMode::Still);
        let (track, fill) = progress_boxes(&context, id);
        assert_eq!(fill, track, "a still bar fills the whole track");
        let accent = nana_ui_core::SemanticPalette::dark().accent;
        let still = context.world().extract_nodes(&[id])[0]
            .standard_visual_foreground
            .unwrap();
        assert!(
            still
                .iter()
                .zip(accent.as_rgba_array())
                .take(3)
                .all(|(still, accent)| (still - accent).abs() < 1e-4),
            "the fill's own hue: {still:?}"
        );
        assert!(
            (still[3] - accent.a * PROGRESS_STILL_ALPHA).abs() < 1e-3,
            "too faint to read as done: {still:?}"
        );
        assert_eq!(context.next_animation_deadline(), None);

        context.set_reduced_motion(false);
        assert!(context.world().animation_is_active(sweep));
        assert!(matches!(
            progress_mode(&context, id),
            ProgressMode::Sweep { .. }
        ));

        context
            .update_component(progress, |progress, _| progress.indeterminate = false)
            .unwrap();
        assert!(!context.world().animation_is_active(sweep));
        assert_eq!(progress_mode(&context, id), ProgressMode::Value);
    }

    /// Parked, a bar's sweep stops with the rest of what the node animates;
    /// mounted again, it sweeps again without the application asking.
    #[test]
    fn a_remounted_indeterminate_bar_sweeps_again() {
        let mut context = AppContext::new();
        let host = context
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let parent = context
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let progress = context
            .create_detached_component(document(), Progress::new(0.0, 1.0).indeterminate(true))
            .unwrap();
        context.append_child(parent, progress).unwrap();
        let pass = context
            .world()
            .theme()
            .duration(nana_ui_core::MotionRole::SkeletonPulse);
        let sweep = Progress::sweep_animation(progress.stable_id(), Duration::ZERO, pass)
            .unwrap()
            .id;
        assert!(context.world().animation_is_active(sweep));

        let mut park = MutationQueue::new();
        park.park_subtree(parent.stable_id());
        context.commit_mutations(park).unwrap();
        assert!(!context.world().animation_is_active(sweep));

        let mut remount = MutationQueue::new();
        remount.insert(host.stable_id(), parent.stable_id(), None);
        context.commit_mutations(remount).unwrap();
        assert!(context.world().animation_is_active(sweep));
    }

    /// The sweep's segment slides in from the start and out past the end,
    /// cut to the track: nothing shows at either end of a pass.
    #[test]
    fn the_sweep_segment_enters_and_leaves_the_track() {
        let mut context = AppContext::new();
        let progress = context
            .create_component(document(), Progress::new(0.0, 1.0).indeterminate(true))
            .unwrap();
        let id = progress.stable_id();
        layout(&mut context, id, 100.0, 6.0);
        // At a fifth of the pass the segment's lead is 8% short of the
        // start, so 27 of its 35 show.
        for (phase, x, width) in [(0.0, 0.0, 0.0), (0.2, 0.0, 27.0), (1.0, 100.0, 0.0)] {
            let mut sample = MutationQueue::new();
            sample.set_standard_visual(
                id,
                Some(StandardVisual::Progress {
                    value_ratio: 0.0,
                    label: None,
                    cancellable: false,
                    mode: ProgressMode::Sweep { phase },
                    thickness: PROGRESS_GIRTH,
                    fill: None,
                }),
            );
            context.commit_mutations(sample).unwrap();
            let (track, fill) = progress_boxes(&context, id);
            assert!(
                (fill.x - (track.x + x)).abs() < 0.01 && (fill.width - width).abs() < 0.01,
                "phase {phase}: {fill:?}"
            );
        }
        let mut broken = MutationQueue::new();
        broken.set_standard_visual(
            id,
            Some(StandardVisual::Progress {
                value_ratio: 0.0,
                label: None,
                cancellable: false,
                mode: ProgressMode::Sweep { phase: f32::NAN },
                thickness: PROGRESS_GIRTH,
                fill: None,
            }),
        );
        assert!(context.commit_mutations(broken).is_err());
    }
}
