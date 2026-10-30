//! Passive notices: the painted leaf [`Toast`], and the composite form
//! [`AppContext::assemble_toast`] builds out of child nodes.

use std::sync::Arc;

use nana_ui_core::framework_strings::fill;
use nana_ui_core::{
    AlignSpec, ButtonKind, ControlSize, FlexDirection, FontFeatureSetting, Icon, LengthSpec,
    LineHeightSpec, PopoverAlignment, PositionSpec, RecipeText, SemanticColorRole, ToastRecipe,
};

use crate::panel::{origin, positive};
use crate::view_components::project_common;
use crate::{
    AccessibilityLive, AccessibilityRole, AccessibilityState, Activate, AppContext, Button,
    ComponentView, Entity, FocusWithinChanged, FrameworkError, IconButton, InteractionState,
    LayoutBox, MutationQueue, NodeKind, NodeStyle, PanelInsets, PointerHoverChanged, Progress,
    StableNodeId, Stack, Text, TextContent, TextHorizontalAlignment, UiWorld,
};

fn sanitize_description(description: Option<&Arc<str>>) -> Option<Arc<str>> {
    description.filter(|value| !value.is_empty()).cloned()
}

fn inert() -> InteractionState {
    InteractionState {
        pointer_events: false,
        focusable: false,
    }
}

fn dismiss_target() -> InteractionState {
    InteractionState {
        pointer_events: true,
        focusable: true,
    }
}

/// Dismiss request: the dismiss button was pressed, or the toast's
/// [`Toast::timeout`] ran out. Removing the toast is the host's call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastDismissed;

/// The toast's action button ([`Toast::action`]) was pressed. What it does
/// is the host's call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastAction;

/// How far the task a toast reports on has got ([`Toast::progress`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToastProgress {
    /// This share of it is done, `0.0..=1.0`: a value outside is clamped,
    /// one that is not a number reads as none done.
    Determinate(f32),
    /// How much is left is unknown: a segment sweeps along the bar.
    Indeterminate,
}

impl ToastProgress {
    /// The share done, clamped to `0.0..=1.0`; `None` while unknown.
    pub fn ratio(self) -> Option<f32> {
        match self {
            Self::Determinate(ratio) if ratio.is_finite() => Some(ratio.clamp(0.0, 1.0)),
            Self::Determinate(_) => Some(0.0),
            Self::Indeterminate => None,
        }
    }

    fn clamped(self) -> Self {
        self.ratio().map_or(Self::Indeterminate, Self::Determinate)
    }
}

pub use nana_ui_core::ToastTone;

/// A passive notice: a title, an optional description and the tone they
/// are said in. It can offer a dismiss hit target and a [`Toast::timeout`];
/// either one only asks the host to remove it. How it looks is the installed
/// theme's [`ToastRecipe`].
///
/// A bare toast is one painted node, its title and description one line
/// each. [`AppContext::assemble_toast`] builds it out of child nodes for
/// good: copy that wraps, and the parts only that form shows — a progress
/// bar ([`Toast::progress`]), an action button ([`Toast::action`]), a close
/// button that emits [`ToastDismissed`], and an icon in place of the tone
/// glyph ([`Toast::icon`]). A [`crate::ToastStack`] assembles every toast
/// it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub tone: ToastTone,
    pub dismissible: bool,
    /// Ask for dismissal after this long on screen. `None` waits for the user.
    pub timeout: Option<std::time::Duration>,
    /// The task's progress, as a bar under the copy. Assembled toasts only.
    pub progress: Option<ToastProgress>,
    /// The label of the toast's one action button, which emits
    /// [`ToastAction`]. Assembled toasts only.
    pub action: Option<Arc<str>>,
    /// Shown in place of the tone's glyph when the theme's recipe marks
    /// tones with glyphs ([`nana_ui_core::ToastIndicator::Glyph`]); a recipe that marks
    /// them with a dot, or not at all, keeps doing so. Assembled toasts only.
    pub icon: Option<Icon>,
    /// The close button's accessible name; `None` says the framework's
    /// (`toast.close`). Assembled toasts only.
    pub close_label: Option<Arc<str>>,
    pub style: NodeStyle,
    /// The nodes [`AppContext::assemble_toast`] built; `None` for a bare
    /// toast.
    pub(crate) parts: Option<ToastParts>,
}

impl Toast {
    pub fn new(title: impl Into<Arc<str>>, tone: ToastTone) -> Self {
        Self {
            title: title.into(),
            description: None,
            tone,
            dismissible: false,
            timeout: None,
            progress: None,
            action: None,
            icon: None,
            close_label: None,
            style: NodeStyle::default(),
            parts: None,
        }
    }

    /// Emit [`ToastDismissed`] once the toast has been on screen this long.
    /// An assembled toast does not count while a pointer is over it or focus
    /// is inside it, and goes on with the time it had left; a toast in a
    /// [`crate::ToastStack`] counts only while the stack shows it and holds
    /// while the pointer or focus is on any toast of the stack.
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Show the task's progress in a bar under the copy, with the share
    /// done spelled out beside the title when the recipe asks for it.
    /// Assembled toasts only.
    pub fn progress(mut self, progress: ToastProgress) -> Self {
        self.progress = Some(progress.clamped());
        self
    }

    /// Offer one action button labelled `label`; pressing it emits
    /// [`ToastAction`]. Assembled toasts only.
    pub fn action(mut self, label: impl Into<Arc<str>>) -> Self {
        self.action = Some(label.into());
        self
    }

    /// Show `icon` in place of the tone's glyph; see [`Self::icon`](field@Self::icon).
    /// Assembled toasts only.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Name the close button for assistive technology. Assembled toasts only.
    pub fn close_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.close_label = Some(label.into());
        self
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        let description = description.into();
        self.description = (!description.is_empty()).then_some(description);
        self
    }

    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Pin the toast into the space `insets` leaves free in `viewport`, using
    /// the same chrome reservation contract as [`crate::Panel::viewport`].
    /// `align` distributes it across that free width and `max_width` caps it.
    /// The toast keeps its intrinsic height and rests on the reserved bottom
    /// edge, so `viewport` must be the positioned host's own box. This updates
    /// geometry only; authored padding and visual style survive.
    pub fn place_in(
        &mut self,
        viewport: LayoutBox,
        align: PopoverAlignment,
        max_width: f32,
        insets: PanelInsets,
    ) {
        let insets = insets.clamp_into(viewport);
        let free = positive(viewport.width) - insets.left - insets.right;
        let width = positive(max_width).min(free);
        let leading = match align {
            PopoverAlignment::Start => 0.0,
            PopoverAlignment::Center => (free - width) / 2.0,
            PopoverAlignment::End => free - width,
        };
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_left = Some(LengthSpec::Px(origin(viewport.x) + insets.left + leading));
        layout.offset_right = None;
        layout.offset_top = None;
        layout.offset_bottom = Some(LengthSpec::Px(insets.bottom));
        layout.width = Some(LengthSpec::Px(width));
    }

    pub fn tone_role(&self) -> SemanticColorRole {
        crate::components::status_tone_role(self.tone.status())
    }

    pub fn resolved_description(&self) -> Option<Arc<str>> {
        sanitize_description(self.description.as_ref())
    }

    /// A bare toast's own box. Its mark, copy and dismiss target are
    /// content parts (`UiWorld::toast_inputs`): measure gives the box their
    /// height, so the style keeps no copy of it.
    fn effective_style(&self, world: &UiWorld) -> NodeStyle {
        let recipe = world.theme().recipes().toast();
        let metrics = world.theme_metrics();
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(recipe.background);
        style.border = recipe.border;
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        let layout = Arc::make_mut(&mut style.layout);
        // A placed toast owns its width; only an unplaced one fills its row.
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::Fill);
        }
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.gap = Some(LengthSpec::Px(recipe.gap));
        let padding = recipe.padding;
        layout.padding_left = Some(LengthSpec::Px(padding.inline));
        layout.padding_right = Some(LengthSpec::Px(padding.inline));
        layout.padding_top = Some(LengthSpec::Px(padding.top));
        layout.padding_bottom = Some(LengthSpec::Px(padding.bottom));
        layout.border_width = recipe.border.map(|_| world.theme().border().hairline);
        layout.border_radius = Some(recipe.radius.resolve(metrics));
        layout.font_size = Some(recipe.title.size);
        if let Some(weight) = recipe.title.weight {
            layout.font_weight = Some(weight);
        }
        style
    }

    /// An assembled toast's own box: the surface its parts stand on, a row
    /// as wide as they need.
    fn assembled_style(&self, world: &UiWorld) -> NodeStyle {
        let recipe = world.theme().recipes().toast();
        let metrics = world.theme_metrics();
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(recipe.background);
        style.border = recipe.border;
        let layout = Arc::make_mut(&mut style.layout);
        // As wide as its copy needs, up to what its place allows; a placed
        // toast keeps the width it was given.
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::FitContent);
        }
        // A toast showing progress is never so narrow that its bar shrinks
        // and grows with the copy.
        if self.progress.is_some() && layout.min_width.is_none() {
            layout.min_width = Some(LengthSpec::Px(recipe.progress.min_width));
        }
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Start;
        layout.gap = Some(LengthSpec::Px(recipe.gap));
        // Its tone mark is a part laid out before the copy, which is its
        // children (#385); the padding is the recipe's alone.
        let padding = recipe.padding;
        layout.padding_left = Some(LengthSpec::Px(padding.inline));
        layout.padding_right = Some(LengthSpec::Px(padding.inline));
        layout.padding_top = Some(LengthSpec::Px(padding.top));
        layout.padding_bottom = Some(LengthSpec::Px(padding.bottom));
        layout.border_width = recipe.border.map(|_| world.theme().border().hairline);
        layout.border_radius = Some(recipe.radius.resolve(metrics));
        style
    }

    fn project_assembled(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = crate::StandardVisual::Toast {
            title: Arc::clone(&self.title),
            description: self.resolved_description(),
            tone: self.tone,
            dismissible: self.dismissible,
            assembled: true,
            icon: self.icon,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        // The copy is child text. A toast assembled after it was shown bare
        // drops the title it painted itself.
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.assembled_style(world),
            // The whole surface takes the pointer, so a press on it does not
            // fall through and hovering anywhere on it holds the timer; the
            // buttons inside take focus, the toast does not.
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Status,
                label: Some(Arc::clone(&self.title)),
                description: self.resolved_description(),
                ..AccessibilityState::default()
            },
        );
        // A title rewritten in place (a task that finished) is announced by
        // the platform's live region; danger interrupts.
        let live = if self.tone == ToastTone::Danger {
            AccessibilityLive::Assertive
        } else {
            AccessibilityLive::Polite
        };
        if world.live(id) != Some(live) {
            mutations.set_live(id, Some(live));
        }
    }
}

/// The line box an assembled toast sets text of `size` in: the control line
/// that size belongs to, never shorter than the text. Its tone mark stands in
/// the title's first one.
pub(crate) fn line_box(size: f32) -> f32 {
    ControlSize::nearest_text(size).line_height().max(size)
}

/// The nodes [`AppContext::assemble_toast`] builds a toast out of, every one
/// of them for good: a part a toast does not show is hidden, not despawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToastParts {
    /// The copy and the bar, in a column beside the buttons.
    pub(crate) copy: StableNodeId,
    /// The title and the percentage, in a row.
    pub(crate) head: StableNodeId,
    pub(crate) title: StableNodeId,
    pub(crate) percent: StableNodeId,
    pub(crate) description: StableNodeId,
    pub(crate) progress: StableNodeId,
    pub(crate) action: StableNodeId,
    pub(crate) close: StableNodeId,
}

impl ToastParts {
    fn all(self) -> [StableNodeId; 8] {
        [
            self.copy,
            self.head,
            self.title,
            self.percent,
            self.description,
            self.progress,
            self.action,
            self.close,
        ]
    }
}

/// Copy set in `text`'s type and `role`, wrapping, `lines` lines at most
/// (0: as many as it takes).
fn copy_style(text: RecipeText, role: SemanticColorRole, lines: u16) -> NodeStyle {
    let mut style = NodeStyle {
        foreground: Some(role),
        ..NodeStyle::default()
    };
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(text.size);
    layout.font_weight = text.weight;
    layout.line_height = Some(LineHeightSpec::Absolute(line_box(text.size)));
    layout.line_clamp = (lines > 0).then_some(lines);
    style
}

/// A button beside the copy: centred on the toast's content, including its
/// description and progress, without making a one-line toast any taller.
fn beside_copy(
    layout: &mut nana_ui_core::LayoutStyle,
    recipe: &ToastRecipe,
    control: f32,
    shown: bool,
) {
    let pull = ((line_box(recipe.title.size) - control) / 2.0).min(0.0);
    layout.margin_top = Some(LengthSpec::Px(pull));
    layout.margin_bottom = Some(LengthSpec::Px(pull));
    layout.align_self = Some(AlignSpec::Center);
    layout.flex_shrink = Some(0.0);
    layout.hidden = !shown;
}

impl AppContext {
    /// Build `toast` out of child nodes, for good.
    ///
    /// Its title and description become text that wraps up to the recipe's
    /// line counts, and it gains the parts a bare toast does not have: a bar
    /// for [`Toast::progress`] (with the percentage beside the title when the
    /// recipe asks for it), a button for [`Toast::action`] that emits
    /// [`ToastAction`], a close button while it is
    /// [`dismissible`](Toast::dismissible) that emits [`ToastDismissed`], and
    /// [`Toast::icon`] in place of the tone glyph. The toast itself paints
    /// only its surface and tone mark, and is as wide as its content up to
    /// its place (a placed toast keeps its width; one showing progress is at
    /// least the recipe's `progress.min_width`).
    ///
    /// Every later write to the toast reconciles those parts in place: a
    /// progress update moves the same bar. Its timer holds while a pointer
    /// is over the toast or focus is inside it. Idempotent: `Ok(true)` when
    /// this call assembled it, `Ok(false)` when it already was.
    pub fn assemble_toast(&mut self, toast: Entity<Toast>) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(toast.stable_id())
            .ok_or(FrameworkError::MissingView(toast.stable_id()))?
            .document;
        if self.read(toast, |toast| toast.parts.is_some())? {
            self.reconcile_toast(toast)?;
            return Ok(false);
        }
        let close_label = Arc::clone(&self.world().framework_strings().toast_close);
        let parts = ToastParts {
            copy: self
                .create_detached_component(document, Stack::column(0.0))?
                .stable_id(),
            head: self
                .create_detached_component(document, Stack::row(0.0))?
                .stable_id(),
            title: self
                .create_detached_component(document, Text::new("").decorative())?
                .stable_id(),
            percent: self
                .create_detached_component(document, Text::new("").decorative())?
                .stable_id(),
            description: self
                .create_detached_component(document, Text::new("").decorative())?
                .stable_id(),
            progress: self
                .create_detached_component(document, Progress::new(0.0, 1.0))?
                .stable_id(),
            action: self
                .create_detached_component(document, Button::new(""))?
                .stable_id(),
            close: self
                .create_detached_component(document, IconButton::new(Icon::Close, close_label))?
                .stable_id(),
        };
        self.observe(
            Entity::<Button>::from_stable_id(parts.action),
            toast,
            |_, _: &Activate, cx| cx.emit(ToastAction),
        )?;
        self.observe(
            Entity::<IconButton>::from_stable_id(parts.close),
            toast,
            |_, _: &Activate, cx| cx.emit(ToastDismissed),
        )?;
        // Heard so the timer can hold while the pointer or focus is on the
        // toast: the write that delivers either reconciles it.
        self.on(toast, |_, _: &PointerHoverChanged, _| {})?;
        self.on(toast, |_, _: &FocusWithinChanged, _| {})?;
        self.update_component(toast, |toast, _| toast.parts = Some(parts))?;
        Ok(true)
    }

    /// [`Toast`]'s assembler: bring an assembled toast's parts in line with
    /// its fields and the installed recipe after every write. Nothing for a
    /// bare toast.
    pub(crate) fn reconcile_toast(&mut self, toast: Entity<Toast>) -> Result<bool, FrameworkError> {
        let Some(snapshot) = self.read(toast, |toast| toast.parts.map(|_| toast.clone()))? else {
            return Ok(false);
        };
        let Some(parts) = snapshot.parts else {
            return Ok(false);
        };
        let world = self.world();
        // Assembly is for good: a part something else removed is not built
        // again.
        if parts.all().iter().any(|id| !world.contains(*id)) {
            return Ok(false);
        }
        let recipe = *world.theme().recipes().toast();
        let control = recipe.control_size.height_in(world.theme_metrics());
        let strings = world.framework_strings();
        let close_label = snapshot
            .close_label
            .clone()
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| Arc::clone(&strings.toast_close));
        let ratio = snapshot.progress.and_then(ToastProgress::ratio);
        let percent = ratio.filter(|_| recipe.progress.show_percent).map(|ratio| {
            let percent = (ratio * 100.0).round() as u32;
            fill(&strings.toast_percent, &[("percent", &percent)])
        });
        // As wide as the widest it gets, so the title beside it stays put as
        // the number grows.
        let widest = fill(&strings.toast_percent, &[("percent", &100)]);
        let percent_text = recipe.progress.percent;
        let percent_width = world.chrome_text_measure(parts.percent).width(
            &widest,
            percent_text.size,
            percent_text.weight,
        );
        let description = snapshot.resolved_description();
        let action = snapshot.action.clone().filter(|label| !label.is_empty());

        // The copy takes the room the buttons leave, down to nothing, so it
        // wraps instead of pushing them out.
        let copy_column = Stack::column(recipe.copy_gap)
            .with_layout(|layout| {
                layout.width = None;
                layout.flex_grow = Some(1.0);
                layout.flex_shrink = Some(1.0);
            })
            .node_style();
        self.update_component(Entity::<Stack>::from_stable_id(parts.copy), |stack, _| {
            *stack.style_mut() = copy_column;
        })?;
        let head_row = Stack::row(recipe.gap)
            .align(AlignSpec::Start)
            .with_layout(|layout| layout.width = None)
            .node_style();
        self.update_component(Entity::<Stack>::from_stable_id(parts.head), |stack, _| {
            *stack.style_mut() = head_row;
        })?;
        self.update_component(Entity::<Text>::from_stable_id(parts.title), |text, _| {
            if text.value != *snapshot.title {
                text.value = snapshot.title.to_string();
            }
            let mut style = copy_style(
                recipe.title,
                SemanticColorRole::Text,
                recipe.title_max_lines,
            );
            let layout = Arc::make_mut(&mut style.layout);
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
            layout.min_width = Some(LengthSpec::Px(0.0));
            text.style = style;
        })?;
        self.update_component(Entity::<Text>::from_stable_id(parts.percent), |text, _| {
            let value = percent.as_deref().unwrap_or_default();
            if text.value != value {
                text.value = value.to_owned();
            }
            let mut style = copy_style(recipe.progress.percent, recipe.progress.percent_role, 0);
            style.text_horizontal_alignment = TextHorizontalAlignment::End;
            let layout = Arc::make_mut(&mut style.layout);
            layout.white_space_nowrap = true;
            layout.flex_shrink = Some(0.0);
            layout.min_width = Some(LengthSpec::Px(percent_width));
            // Tabular digits where the font has them: every number as wide.
            layout.font_features = Some(vec![FontFeatureSetting::new(*b"tnum", 1)]);
            layout.hidden = percent.is_none();
            text.style = style;
        })?;
        self.update_component(
            Entity::<Text>::from_stable_id(parts.description),
            |text, _| {
                let value = description.as_deref().unwrap_or_default();
                if text.value != value {
                    text.value = value.to_owned();
                }
                let mut style = copy_style(
                    recipe.description,
                    SemanticColorRole::Muted,
                    recipe.description_max_lines,
                );
                Arc::make_mut(&mut style.layout).hidden = description.is_none();
                text.style = style;
            },
        )?;
        self.update_component(
            Entity::<Progress>::from_stable_id(parts.progress),
            |bar, _| {
                bar.value = f64::from(ratio.unwrap_or(0.0));
                bar.indeterminate = snapshot.progress == Some(ToastProgress::Indeterminate);
                bar.thickness = Some(recipe.progress.height);
                bar.track = Some(recipe.progress.track);
                bar.fill = Some(recipe.progress.fill);
                let layout = Arc::make_mut(&mut bar.style.layout);
                // Stretched across the copy column, not filling whatever the
                // toast stands in: the bar must not make the toast wider.
                layout.width = Some(LengthSpec::Auto);
                layout.margin_top = Some(LengthSpec::Px(recipe.progress.top_gap));
                layout.hidden = snapshot.progress.is_none();
            },
        )?;
        self.update_component(
            Entity::<Button>::from_stable_id(parts.action),
            |button, _| {
                let mut next = button
                    .clone()
                    .kind(recipe.action_kind)
                    .size(recipe.control_size);
                next.style.radius = Some(recipe.control_radius);
                if let Some(label) = &action
                    && next.label != **label
                {
                    next.label = label.to_string();
                }
                beside_copy(
                    Arc::make_mut(&mut next.style.layout),
                    &recipe,
                    control,
                    action.is_some(),
                );
                *button = next;
            },
        )?;
        self.update_component(
            Entity::<IconButton>::from_stable_id(parts.close),
            |button, _| {
                let mut next = button
                    .clone()
                    .kind(ButtonKind::Text)
                    .size(recipe.control_size);
                next.style.radius = Some(recipe.control_radius);
                if next.label != close_label {
                    next.label = Arc::clone(&close_label);
                }
                beside_copy(
                    Arc::make_mut(&mut next.style.layout),
                    &recipe,
                    control,
                    snapshot.dismissible,
                );
                *button = next;
            },
        )?;
        self.append_children(parts.head, &[parts.title, parts.percent])?;
        self.append_children(parts.copy, &[parts.head, parts.description, parts.progress])?;
        self.append_children(toast.stable_id(), &[parts.copy, parts.action, parts.close])?;

        // The bar is named by the title; the parts that change while the
        // toast shows are not announced on their own, only the toast is.
        let world = self.world();
        let mut relations = MutationQueue::new();
        if world.labelled_by(parts.progress) != Some(parts.title) {
            relations.set_labelled_by(parts.progress, Some(parts.title));
        }
        for part in [parts.progress, parts.action, parts.close] {
            if world.live(part) != Some(AccessibilityLive::Off) {
                relations.set_live(part, Some(AccessibilityLive::Off));
            }
        }
        if !relations.is_empty() {
            self.commit_mutations(relations)?;
        }
        self.sync_toast_timer(toast.stable_id())?;
        Ok(true)
    }
}

impl ComponentView for Toast {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::reconcile_toast),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// A toast rebuilt from declared props stays assembled.
    fn reconcile(&mut self, next: Self) {
        let parts = self.parts;
        *self = next;
        self.parts = self.parts.or(parts);
    }

    /// A toast written whole (`*toast = Toast::new(..)`) stays assembled.
    fn keep_framework_state(&mut self, previous: &Self) {
        self.parts = self.parts.or(previous.parts);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "toast".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    /// The toast recipe decides the surface, insets and type `project`
    /// authors onto the node; only a reprojection carries a new one to a
    /// toast already on screen.
    fn wants_recipe_reproject() -> bool {
        true
    }

    /// An assembled toast sizes its percentage by measuring the widest one;
    /// measured before any engine shaped, that is an estimate.
    fn wants_text_backend_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if self.parts.is_some() {
            self.project_assembled(id, world, mutations);
            return;
        }
        let visual = crate::StandardVisual::Toast {
            title: Arc::clone(&self.title),
            description: self.resolved_description(),
            tone: self.tone,
            dismissible: self.dismissible,
            assembled: false,
            icon: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.text(id) != Some(self.title.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.title.to_string().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world),
            if self.dismissible {
                dismiss_target()
            } else {
                inert()
            },
            AccessibilityState {
                role: AccessibilityRole::Status,
                label: Some(Arc::clone(&self.title)),
                description: self.resolved_description(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;
    use std::sync::{Arc, Mutex};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn toast_constructs_and_maps_tone() {
        let empty = Toast::new("Saved", ToastTone::Success).description("");
        assert!(empty.description.is_none());
        assert_eq!(empty.resolved_description(), None);
        assert_eq!(ToastTone::Info.status(), nana_ui_core::StatusTone::Info);
        assert_eq!(
            Toast::new("A", ToastTone::Info).tone_role(),
            SemanticColorRole::Accent
        );
        assert_eq!(
            Toast::new("A", ToastTone::Success).tone_role(),
            SemanticColorRole::Success
        );
        assert_eq!(
            Toast::new("A", ToastTone::Warning).tone_role(),
            SemanticColorRole::Warning
        );
        assert_eq!(
            Toast::new("A", ToastTone::Danger).tone_role(),
            SemanticColorRole::Danger
        );

        let mut context = AppContext::new();
        let toast = context
            .create_component(
                document(),
                Toast::new("Saved", ToastTone::Success).description("Project exported"),
            )
            .unwrap();
        let id = toast.stable_id();

        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "toast"
        ));
        assert!(matches!(
            context.world().standard_visual(id),
            Some(crate::StandardVisual::Toast {
                tone: ToastTone::Success,
                dismissible: false,
                ..
            })
        ));
        assert_eq!(context.world().text(id), Some("Saved"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.border, Some(SemanticColorRole::Border));
        assert_eq!(style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(style.layout.border_width, Some(nana_ui_core::HAIRLINE));
        assert_eq!(
            style.layout.border_radius,
            Some(context.world().theme_metrics().radius_md)
        );
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        // The built-in recipe's insets and type are the numbers the toast
        // always had: 12 across, 10 down, a 12px semibold title over 11px.
        assert_eq!(style.layout.padding_left, Some(LengthSpec::Px(12.0)));
        assert_eq!(style.layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert_eq!(style.layout.padding_top, Some(LengthSpec::Px(10.0)));
        assert_eq!(style.layout.padding_bottom, Some(LengthSpec::Px(10.0)));
        assert_eq!(style.layout.gap, Some(LengthSpec::Px(8.0)));
        assert_eq!(style.layout.font_size, Some(12.0));
        assert_eq!(style.layout.font_weight, Some(600));
        // Its height is measured from its parts (#205): the insets, a 12px
        // title over an 11px description 2 apart.
        assert_eq!(style.layout.min_height, None);
        assert_eq!(context.world().interaction(id), Some(inert()));
        lay_out(&mut context, 420.0);
        assert_eq!(
            context.world().layout_box(id).unwrap().height,
            10.0 + 12.0 + 2.0 + 11.0 + 10.0
        );
    }

    #[test]
    fn toast_dismissible_vs_inert() {
        let mut context = AppContext::new();
        let toast = context
            .create_component(document(), Toast::new("Copied", ToastTone::Info))
            .unwrap();
        let id = toast.stable_id();
        let recipe = ToastRecipe::DEFAULT;
        let (pad_top, pad_bottom) = (recipe.padding.top, recipe.padding.bottom);
        let dismiss = nana_ui_core::ControlSize::Small.height_in(context.world().theme_metrics());

        assert!(!context.read(toast, |toast| toast.dismissible).unwrap());
        assert_eq!(context.world().interaction(id), Some(inert()));
        // Its height is measured from its parts (#205): at least the insets
        // and the title or the mark, and at least the one-line title in its
        // content box.
        lay_out(&mut context, 420.0);
        let border = nana_ui_core::HAIRLINE * 2.0;
        let line = context.world().text_metrics(id).unwrap().height;
        assert_eq!(
            context.world().layout_box(id).unwrap().height,
            (pad_top + recipe.title.size.max(recipe.indicator.size()) + pad_bottom)
                .max(pad_top + border + line + pad_bottom)
        );

        context
            .update_component(toast, |toast, _| {
                toast.dismissible = true;
            })
            .unwrap();
        assert_eq!(context.world().interaction(id), Some(dismiss_target()));
        lay_out(&mut context, 420.0);
        assert_eq!(
            context.world().layout_box(id).unwrap().height,
            pad_top + dismiss + pad_bottom
        );
        assert_eq!(
            context.world().node_style(id).unwrap().layout.min_height,
            None
        );

        let dismissed = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&dismissed);
        context
            .on(toast, move |_toast, _event: &ToastDismissed, _cx| {
                *flag.lock().unwrap() = true;
            })
            .unwrap();
        context
            .update_component(toast, |_toast, cx| {
                cx.emit(ToastDismissed);
            })
            .unwrap();
        assert!(*dismissed.lock().unwrap());

        context
            .update_component(toast, |toast, _| {
                toast.dismissible = false;
            })
            .unwrap();
        assert_eq!(context.world().interaction(id), Some(inert()));
    }

    #[test]
    fn timed_toast_asks_for_dismissal_once_after_its_timeout() {
        use std::time::Duration;

        let mut context = AppContext::new();
        let timed = context
            .create_component(
                document(),
                Toast::new("Saved", ToastTone::Success).timeout(Duration::from_secs(4)),
            )
            .unwrap();
        let sticky = context
            .create_component(document(), Toast::new("Sync failed", ToastTone::Danger))
            .unwrap();
        let dismissals = Arc::new(Mutex::new(Vec::new()));
        for (toast, name) in [(timed, "timed"), (sticky, "sticky")] {
            let log = Arc::clone(&dismissals);
            context
                .on(toast, move |_toast, _event: &ToastDismissed, _cx| {
                    log.lock().unwrap().push(name);
                })
                .unwrap();
        }

        // The first frame comes long after the toast mounted: the timeout
        // counts from that frame, not from the stale mount clock.
        context.advance_animations(Duration::from_secs(100));
        assert!(dismissals.lock().unwrap().is_empty());
        context.advance_animations(Duration::from_secs(103));
        assert!(dismissals.lock().unwrap().is_empty());
        context.advance_animations(Duration::from_secs(104));
        assert_eq!(*dismissals.lock().unwrap(), ["timed"]);
        context.advance_animations(Duration::from_secs(300));
        assert_eq!(*dismissals.lock().unwrap(), ["timed"]);
    }

    #[test]
    fn a_toast_built_detached_starts_its_timer_once_it_is_attached() {
        use std::time::Duration;

        let mut context = AppContext::new();
        let host = context
            .create_component(document(), crate::OverlayHost::new())
            .unwrap();
        let toast = context
            .create_detached_component(
                document(),
                Toast::new("Copied", ToastTone::Success).timeout(Duration::from_secs(3)),
            )
            .unwrap();
        let dismissed = Arc::new(Mutex::new(0));
        let count = Arc::clone(&dismissed);
        context
            .on(toast, move |_toast, _event: &ToastDismissed, _cx| {
                *count.lock().unwrap() += 1;
            })
            .unwrap();
        context.advance_animations(Duration::from_secs(10));
        assert_eq!(*dismissed.lock().unwrap(), 0);

        context.append_child(host, toast).unwrap();
        context.advance_animations(Duration::from_secs(12));
        context.advance_animations(Duration::from_secs(14));
        assert_eq!(*dismissed.lock().unwrap(), 0);
        context.advance_animations(Duration::from_secs(15));
        assert_eq!(*dismissed.lock().unwrap(), 1);
    }

    #[test]
    fn toast_accessibility_label() {
        let mut context = AppContext::new();
        let toast = context
            .create_component(
                document(),
                Toast::new("Sync failed", ToastTone::Danger).description("Network offline"),
            )
            .unwrap();
        let id = toast.stable_id();
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Status);
        assert_eq!(accessibility.label.as_deref(), Some("Sync failed"));
        assert_eq!(
            accessibility.description.as_deref(),
            Some("Network offline")
        );

        context
            .update_component(toast, |toast, _| {
                toast.description = Some(Arc::from(""));
            })
            .unwrap();
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("Sync failed"));
        assert_eq!(accessibility.description, None);
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let toast = context
            .create_component(
                document(),
                Toast::new("Ready", ToastTone::Info)
                    .description("Idle")
                    .dismissible(true),
            )
            .unwrap();
        let _ = context.take_system_work();
        context.reproject_component(toast).unwrap();
        assert!(context.take_system_work().is_empty());
    }

    #[test]
    fn place_in_centres_within_reserved_chrome_and_survives_a_tiny_window() {
        let mut toast = Toast::new("Saved", ToastTone::Info);
        let insets = crate::PanelInsets {
            top: 48.0,
            right: 416.0,
            bottom: 100.0,
            left: 16.0,
        };
        let viewport = crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        toast.place_in(viewport, PopoverAlignment::Center, 360.0, insets);
        let layout = &toast.style.layout;
        assert_eq!(layout.position, PositionSpec::Absolute);
        assert_eq!(layout.width, Some(LengthSpec::Px(360.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(120.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Px(100.0)));
        assert_eq!(layout.offset_right, None);

        toast.place_in(viewport, PopoverAlignment::End, 360.0, insets);
        assert_eq!(
            toast.style.layout.offset_left,
            Some(LengthSpec::Px(1000.0 - 416.0 - 360.0))
        );

        let tiny = crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        toast.place_in(tiny, PopoverAlignment::Center, 360.0, insets);
        let layout = &toast.style.layout;
        assert_eq!(layout.width, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(16.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Px(2.0)));
    }

    #[test]
    fn a_placed_toast_keeps_its_width_through_projection() {
        let mut context = AppContext::new();
        let mut toast = Toast::new("Saved", ToastTone::Info);
        toast.place_in(
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            PopoverAlignment::Center,
            360.0,
            crate::PanelInsets::default(),
        );
        let entity = context.create_component(document(), toast).unwrap();
        let style = context.world().node_style(entity.stable_id()).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::Px(360.0)));
    }

    /// A theme's toast recipe reaches a toast already on screen: its
    /// surface, stroke and type on the node, its lift, mark and type in the
    /// geometry the scene paints. The stroke follows the theme's hairline,
    /// so a theme that moves only the hairline moves it too.
    #[test]
    fn an_installed_toast_recipe_reaches_a_toast_that_is_already_on_screen() {
        use nana_ui_core::{
            ElevationRole, Icon, RadiusTier, RecipeText, ThemeDefinition, ToastGlyphs,
            ToastIndicator,
        };
        let mut context = AppContext::new();
        let toast = context
            .create_component(
                document(),
                Toast::new("Saved", ToastTone::Success).description("Project exported"),
            )
            .unwrap();
        let id = toast.stable_id();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 360.0,
                height: 48.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let geometry = |context: &AppContext| match context.world().component_geometry(id) {
            Some(crate::ComponentGeometry::Toast {
                indicator,
                glyph,
                title,
                elevation,
                ..
            }) => (
                indicator,
                glyph,
                title.expect("a bare toast paints its title"),
                elevation,
            ),
            other => panic!("toast geometry, got {other:?}"),
        };
        let (indicator, glyph, title, elevation) = geometry(&context);
        assert_eq!(indicator.map(|mark| mark.width), Some(6.0));
        assert_eq!(glyph, None, "the built-in mark is a dot");
        assert_eq!(elevation, None, "the built-in toast lies flat");
        assert_eq!(title.bounds.x, 12.0 + 6.0 + 8.0);

        let glyphs = ToastGlyphs {
            info: Icon::About,
            success: Icon::ShieldCheck,
            warning: Icon::Eye,
            danger: Icon::Close,
        };
        let dressed = ToastRecipe {
            radius: RadiusTier::Xl,
            border: Some(SemanticColorRole::BorderSoft),
            elevation: Some(ElevationRole::Surface),
            indicator: ToastIndicator::Glyph { size: 16.0, glyphs },
            title: RecipeText::new(13.0, Some(500)),
            ..ToastRecipe::DEFAULT
        };
        let definition = ThemeDefinition::NANA_DARK.with_toast(dressed);
        assert!(context.set_theme_definition(&definition).unwrap());

        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
        assert_eq!(style.layout.border_width, Some(nana_ui_core::HAIRLINE));
        assert_eq!(
            style.layout.border_radius,
            Some(context.world().theme_metrics().radius_xl)
        );
        assert_eq!(style.layout.font_size, Some(13.0));
        assert_eq!(style.layout.font_weight, Some(500));
        assert_eq!(style.layout.min_height, None);
        let (indicator, glyph, title, elevation) = geometry(&context);
        assert_eq!(indicator.map(|mark| mark.width), Some(16.0));
        assert_eq!(glyph, Some(Icon::ShieldCheck), "the success tone's glyph");
        assert_eq!(title.font_size, 13.0);
        assert_eq!(title.font_weight, Some(500));
        assert_eq!(title.bounds.x, 12.0 + 16.0 + 8.0);
        assert_eq!(
            elevation,
            Some(crate::ComponentElevation::from_shadow(
                context.world().theme().shadow(ElevationRole::Surface)
            ))
        );

        let mut thicker = definition.clone();
        thicker.tokens.border.hairline = 2.0;
        assert!(context.set_theme_definition(&thicker.bump()).unwrap());
        assert_eq!(
            context.world().node_style(id).unwrap().layout.border_width,
            Some(2.0)
        );

        let bare = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            border: None,
            indicator: ToastIndicator::None,
            ..dressed
        });
        assert!(context.set_theme_definition(&bare).unwrap());
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.border, None);
        assert_eq!(style.layout.border_width, None);
        let (indicator, glyph, title, _) = geometry(&context);
        assert_eq!((indicator, glyph), (None, None));
        assert_eq!(
            title.bounds.x, 12.0,
            "no mark, no gap: the copy starts at the inset"
        );
    }

    /// The parts `assemble_toast` built for `toast`.
    fn parts_of(context: &AppContext, toast: Entity<Toast>) -> ToastParts {
        context
            .read(toast, |toast| toast.parts)
            .unwrap()
            .expect("the toast is assembled")
    }

    fn hidden(context: &AppContext, id: StableNodeId) -> bool {
        context.world().node_style(id).unwrap().layout.hidden
    }

    fn children(context: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
        context.world().node(id).unwrap().children.to_vec()
    }

    fn assembled(context: &mut AppContext, toast: Toast) -> Entity<Toast> {
        let toast = context.create_component(document(), toast).unwrap();
        assert!(context.assemble_toast(toast).unwrap());
        toast
    }

    /// A host frame: styles, text shaped, layout, text shaped again at the
    /// widths layout gave it, layout again.
    fn lay_out(context: &mut AppContext, width: f32) {
        let viewport = crate::LayoutViewport::new(width, 600.0);
        let mut shaper = crate::MeasureTextShaper;
        for _ in 0..2 {
            let work = context.take_system_work();
            context.resolve_styles(&work.style).unwrap();
            context.shape_text(&work.text, &mut shaper).unwrap();
            context.layout_document(document(), viewport).unwrap();
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap();
            context.layout_document(document(), viewport).unwrap();
        }
    }

    fn text_of(context: &AppContext, id: StableNodeId) -> String {
        context
            .read(Entity::<crate::Text>::from_stable_id(id), |text| {
                text.value.clone()
            })
            .unwrap()
    }

    /// A theme can round just the controls without rebuilding them or
    /// changing their size, action kind, or toast surface radius.
    #[test]
    fn control_radius_follows_theme_changes_on_the_existing_buttons() {
        use nana_ui_core::{RadiusTier, ThemeDefinition, ToastRecipe};

        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Saved", ToastTone::Success)
                .action("View")
                .dismissible(true),
        );
        let parts = parts_of(&context, toast);
        for (theme, tier) in [
            (ThemeDefinition::NANA_LIGHT, RadiusTier::Sm),
            (ThemeDefinition::NANA_DARK, RadiusTier::Xl),
            (ThemeDefinition::NANA_LIGHT, RadiusTier::Lg),
            (ThemeDefinition::NANA_DARK, RadiusTier::Sm),
        ] {
            let recipe = ToastRecipe {
                control_radius: tier,
                ..ToastRecipe::DEFAULT
            };
            context
                .set_theme_definition(&theme.with_toast(recipe))
                .unwrap();
            lay_out(&mut context, 360.0);
            let metrics = context.world().theme_metrics();
            for id in [parts.action, parts.close] {
                let style = context.world().node_style(id).unwrap();
                assert_eq!(style.radius, Some(tier));
                assert_eq!(
                    context.world().layout_box(id).unwrap().height,
                    recipe.control_size.height_in(metrics)
                );
            }
            assert_eq!(parts_of(&context, toast), parts);
            assert_eq!(
                context
                    .read(Entity::<Button>::from_stable_id(parts.action), |b| b.kind)
                    .unwrap(),
                recipe.action_kind
            );
            assert_eq!(
                context
                    .read(Entity::<IconButton>::from_stable_id(parts.close), |b| b
                        .kind)
                    .unwrap(),
                ButtonKind::Text
            );
            assert_eq!(
                context
                    .world()
                    .node_style(toast.stable_id())
                    .unwrap()
                    .layout
                    .border_radius,
                Some(recipe.radius.resolve(metrics))
            );
        }
    }

    /// Assembly turns the painted leaf into a row of parts: the copy column
    /// (title and percentage, description, bar), the action, the close
    /// button, in that order; the root paints no text of its own. A part
    /// the toast does not say anything for is there and hidden. Assembling
    /// again changes nothing.
    #[test]
    fn assembling_builds_the_parts_in_order_and_shows_what_the_toast_says() {
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimizing models", ToastTone::Info).description("Library"),
        );
        let id = toast.stable_id();
        let parts = parts_of(&context, toast);
        assert_eq!(
            children(&context, id),
            vec![parts.copy, parts.action, parts.close]
        );
        assert_eq!(
            children(&context, parts.copy),
            vec![parts.head, parts.description, parts.progress]
        );
        assert_eq!(
            children(&context, parts.head),
            vec![parts.title, parts.percent]
        );
        assert_eq!(text_of(&context, parts.title), "Optimizing models");
        assert_eq!(text_of(&context, parts.description), "Library");
        assert!(!hidden(&context, parts.title));
        assert!(!hidden(&context, parts.description));
        for part in [parts.percent, parts.progress, parts.action, parts.close] {
            assert!(hidden(&context, part), "nothing to show in {part:?}");
        }
        assert_eq!(context.world().text(id), Some(""), "no text of its own");
        assert!(matches!(
            context.world().standard_visual(id),
            Some(crate::StandardVisual::Toast {
                assembled: true,
                ..
            })
        ));
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: true,
                focusable: false,
            })
        );
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::FitContent));
        assert_eq!(style.layout.align_items, AlignSpec::Start);
        // The dot and the gap after it are laid out before the copy, not
        // in the padding (#385).
        assert_eq!(style.layout.padding_left, Some(LengthSpec::Px(12.0)));
        assert_eq!(
            context.world().leading_parts(id).map(|lead| lead.extent),
            Some(6.0 + 8.0)
        );

        context
            .update_component(toast, |toast, _| {
                toast.action = Some(Arc::from("View"));
                toast.dismissible = true;
                toast.description = None;
            })
            .unwrap();
        assert_eq!(parts_of(&context, toast), parts, "the same nodes");
        assert!(!hidden(&context, parts.action));
        assert!(!hidden(&context, parts.close));
        assert!(hidden(&context, parts.description));
        let action = Entity::<crate::Button>::from_stable_id(parts.action);
        assert_eq!(
            context.read(action, |button| button.label.clone()).unwrap(),
            "View"
        );
        assert_eq!(
            context.read(action, |button| button.kind).unwrap(),
            ToastRecipe::DEFAULT.action_kind
        );

        assert!(!context.assemble_toast(toast).unwrap(), "already assembled");
        assert_eq!(parts_of(&context, toast), parts);
        assert_eq!(
            children(&context, id),
            vec![parts.copy, parts.action, parts.close]
        );
    }

    /// The percentage stands beside the title only for a known share and a
    /// recipe that spells it out; it is the framework's `toast.percent`.
    #[test]
    fn the_percentage_shows_only_for_a_known_share_the_recipe_spells_out() {
        use nana_ui_core::{ThemeDefinition, ToastProgressRecipe};
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimizing", ToastTone::Info).progress(ToastProgress::Determinate(0.4)),
        );
        let parts = parts_of(&context, toast);
        assert!(!hidden(&context, parts.percent));
        assert_eq!(text_of(&context, parts.percent), "40%");
        let style = context.world().node_style(parts.percent).unwrap();
        assert_eq!(style.foreground, Some(SemanticColorRole::Muted));
        assert_eq!(
            style.layout.font_features,
            Some(vec![FontFeatureSetting::new(*b"tnum", 1)])
        );
        let Some(LengthSpec::Px(width)) = style.layout.min_width else {
            panic!(
                "the percentage keeps a fixed width: {:?}",
                style.layout.min_width
            );
        };
        assert!(width > 0.0);
        assert!(
            context.world().accessibility(parts.percent).unwrap().hidden,
            "the bar says it to assistive technology"
        );

        context
            .update_component(toast, |toast, _| {
                toast.progress = Some(ToastProgress::Determinate(1.7));
            })
            .unwrap();
        assert_eq!(text_of(&context, parts.percent), "100%", "clamped");

        context
            .update_component(toast, |toast, _| {
                toast.progress = Some(ToastProgress::Indeterminate);
            })
            .unwrap();
        assert!(hidden(&context, parts.percent), "no share to spell out");

        let quiet = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            progress: ToastProgressRecipe {
                show_percent: false,
                ..ToastProgressRecipe::DEFAULT
            },
            ..ToastRecipe::DEFAULT
        });
        context
            .update_component(toast, |toast, _| {
                toast.progress = Some(ToastProgress::Determinate(0.4));
            })
            .unwrap();
        assert!(!hidden(&context, parts.percent));
        assert!(context.set_theme_definition(&quiet).unwrap());
        assert!(
            hidden(&context, parts.percent),
            "the recipe keeps the number to the bar"
        );

        let mut strings = nana_ui_core::FrameworkStrings::default();
        strings.set("toast.percent", "{percent} pct");
        context.set_framework_strings(strings).unwrap();
        assert!(
            context
                .set_theme_definition(&ThemeDefinition::NANA_DARK.bump())
                .unwrap()
        );
        assert_eq!(text_of(&context, parts.percent), "40 pct");
    }

    /// Unknown progress is a sweeping bar; the bar is the recipe's, named by
    /// the title, and updating the progress in place moves the same bar.
    #[test]
    fn progress_updates_move_the_same_bar_and_an_unknown_share_sweeps() {
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimizing", ToastTone::Info).progress(ToastProgress::Indeterminate),
        );
        let parts = parts_of(&context, toast);
        let bar = Entity::<Progress>::from_stable_id(parts.progress);
        let read = |context: &AppContext| context.read(bar, Clone::clone).unwrap();
        assert!(!hidden(&context, parts.progress));
        let progress = read(&context);
        assert!(progress.indeterminate);
        let recipe = ToastRecipe::DEFAULT.progress;
        assert_eq!(progress.thickness, Some(recipe.height));
        assert_eq!(progress.track, Some(recipe.track));
        assert_eq!(progress.fill, Some(recipe.fill));
        assert_eq!(
            progress.style.layout.margin_top,
            Some(LengthSpec::Px(recipe.top_gap))
        );
        assert_eq!(
            context.world().labelled_by(parts.progress),
            Some(parts.title)
        );
        assert_eq!(
            context.world().live(parts.progress),
            Some(AccessibilityLive::Off)
        );

        let nodes = context.world().document_order(document()).len();
        for step in 1..=4 {
            context
                .update_component(toast, |toast, _| {
                    toast.progress = Some(ToastProgress::Determinate(step as f32 / 4.0));
                })
                .unwrap();
            assert_eq!(parts_of(&context, toast), parts);
            let progress = read(&context);
            assert!(!progress.indeterminate);
            assert_eq!(progress.value, f64::from(step as f32 / 4.0));
        }
        assert_eq!(
            context.world().document_order(document()).len(),
            nodes,
            "nothing respawned"
        );
        assert!(context.world().contains(parts.progress));

        context
            .update_component(toast, |toast, _| toast.progress = None)
            .unwrap();
        assert!(hidden(&context, parts.progress));
    }

    /// The action button raises `ToastAction` and the close button
    /// `ToastDismissed`, each on the toast; the close button is named by the
    /// framework's `toast.close` unless the toast names it.
    #[test]
    fn the_action_and_close_buttons_raise_the_toast_s_events() {
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimized", ToastTone::Success)
                .action("Open")
                .dismissible(true),
        );
        let parts = parts_of(&context, toast);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&heard);
        context
            .on(toast, move |_, _: &ToastAction, _| {
                log.lock().unwrap().push("action");
            })
            .unwrap();
        let log = Arc::clone(&heard);
        context
            .on(toast, move |_, _: &ToastDismissed, _| {
                log.lock().unwrap().push("dismissed");
            })
            .unwrap();
        let action = Entity::<crate::Button>::from_stable_id(parts.action);
        let close = Entity::<crate::IconButton>::from_stable_id(parts.close);
        assert!(context.activate_button(action).unwrap());
        assert!(context.activate_icon_button(close).unwrap());
        assert_eq!(*heard.lock().unwrap(), ["action", "dismissed"]);

        let label = |context: &AppContext| {
            context
                .world()
                .accessibility(parts.close)
                .and_then(|state| state.label.clone())
        };
        assert_eq!(label(&context).as_deref(), Some("关闭通知"));
        context
            .update_component(toast, |toast, _| {
                toast.close_label = Some(Arc::from("Close notice"));
            })
            .unwrap();
        assert_eq!(label(&context).as_deref(), Some("Close notice"));
        for part in [parts.action, parts.close] {
            assert_eq!(context.world().live(part), Some(AccessibilityLive::Off));
        }
    }

    /// A recipe that marks tones with glyphs paints the toast's icon in the
    /// tone's place; a recipe with a dot keeps its dot.
    #[test]
    fn an_icon_replaces_the_tone_glyph_where_the_recipe_marks_tones_with_glyphs() {
        use nana_ui_core::{ThemeDefinition, ToastGlyphs};
        let glyph = |context: &AppContext, id| match context.world().component_geometry(id) {
            Some(crate::ComponentGeometry::Toast {
                glyph,
                title,
                description,
                dismiss,
                ..
            }) => {
                assert!(title.is_none() && description.is_none() && dismiss.is_none());
                glyph
            }
            other => panic!("toast geometry, got {other:?}"),
        };
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimized", ToastTone::Success)
                .description("All models")
                .icon(Icon::Sparkles)
                .dismissible(true),
        );
        let id = toast.stable_id();
        lay_out(&mut context, 800.0);
        assert_eq!(glyph(&context, id), None, "the built-in mark is a dot");

        let glyphs = ToastGlyphs {
            info: Icon::About,
            success: Icon::ShieldCheck,
            warning: Icon::Eye,
            danger: Icon::Close,
        };
        let marked = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            indicator: nana_ui_core::ToastIndicator::Glyph { size: 16.0, glyphs },
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&marked).unwrap());
        lay_out(&mut context, 800.0);
        assert_eq!(glyph(&context, id), Some(Icon::Sparkles));
        context
            .update_component(toast, |toast, _| toast.icon = None)
            .unwrap();
        lay_out(&mut context, 800.0);
        assert_eq!(glyph(&context, id), Some(Icon::ShieldCheck), "the tone's");
    }

    /// An assembled toast is a status named by its title and described by
    /// its description, announced politely, and at once in danger.
    #[test]
    fn an_assembled_toast_is_a_live_status_that_interrupts_for_danger() {
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Optimizing", ToastTone::Info)
                .description("2 of 5")
                .progress(ToastProgress::Determinate(0.4))
                .action("Open"),
        );
        let id = toast.stable_id();
        let parts = parts_of(&context, toast);
        let state = context.world().accessibility(id).unwrap();
        assert_eq!(state.role, AccessibilityRole::Status);
        assert_eq!(state.label.as_deref(), Some("Optimizing"));
        assert_eq!(state.description.as_deref(), Some("2 of 5"));
        assert_eq!(context.world().live(id), Some(AccessibilityLive::Polite));

        lay_out(&mut context, 800.0);
        let nodes = context.world().project_accessibility(document());
        let spoken = |id| nodes.iter().find(|node| node.id == id);
        assert_eq!(
            spoken(parts.progress).and_then(|node| node.label.as_deref()),
            Some("Optimizing"),
            "the bar is named by the title"
        );
        for copy in [parts.title, parts.description, parts.percent] {
            assert!(spoken(copy).is_none(), "the status says its copy itself");
        }
        assert!(spoken(parts.action).is_some());

        context
            .update_component(toast, |toast, _| toast.tone = ToastTone::Danger)
            .unwrap();
        assert_eq!(context.world().live(id), Some(AccessibilityLive::Assertive));
        // The title rewritten in place is what the live region announces.
        context
            .update_component(toast, |toast, _| toast.title = Arc::from("Failed"))
            .unwrap();
        assert_eq!(
            context.world().accessibility(id).unwrap().label.as_deref(),
            Some("Failed")
        );
    }

    /// The tone mark stands in the first line of a title that wraps, not in
    /// the middle of the toast; the copy wraps to the recipe's line counts.
    #[test]
    fn the_mark_stands_in_the_first_line_of_a_wrapping_title() {
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new(
                "Optimizing every model in the library for faster loading",
                ToastTone::Info,
            )
            .description("Models load faster once they are optimized.")
            .dismissible(true),
        );
        let id = toast.stable_id();
        let parts = parts_of(&context, toast);
        lay_out(&mut context, 220.0);
        let bounds = context.world().layout_box(id).unwrap();
        let title = context.world().layout_box(parts.title).unwrap();
        let line = line_box(ToastRecipe::DEFAULT.title.size);
        assert!(
            title.height > line,
            "the title wraps in a narrow place: {title:?}"
        );
        assert!(bounds.width <= 220.0, "{bounds:?}");
        let Some(crate::ComponentGeometry::Toast {
            indicator: Some(mark),
            ..
        }) = context.world().component_geometry(id)
        else {
            panic!("an assembled toast keeps its mark");
        };
        let mark_centre = mark.y + mark.height / 2.0;
        assert!(
            (mark_centre - (title.y + line / 2.0)).abs() < 0.01,
            "the mark stands in the first line: {mark:?} {title:?}"
        );
        assert!(
            (mark_centre - (bounds.y + bounds.height / 2.0)).abs() > 1.0,
            "not in the middle of a toast that wraps"
        );
        assert!(mark.x + mark.width <= title.x - ToastRecipe::DEFAULT.gap + 0.01);

        // The close button is centred on the whole toast, even as the copy wraps.
        let close = context.world().layout_box(parts.close).unwrap();
        assert!(
            ((close.y + close.height / 2.0) - (bounds.y + bounds.height / 2.0)).abs() < 0.01,
            "{close:?} {bounds:?}"
        );

        // The built-in recipe clamps the title and the description to a
        // line each; zero does not clamp.
        let clamp =
            |context: &AppContext, id| context.world().node_style(id).unwrap().layout.line_clamp;
        assert_eq!(clamp(&context, parts.title), Some(1));
        assert_eq!(clamp(&context, parts.description), Some(1));
        let wide = nana_ui_core::ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            title_max_lines: 2,
            description_max_lines: 0,
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&wide).unwrap());
        assert_eq!(clamp(&context, parts.title), Some(2));
        assert_eq!(clamp(&context, parts.description), None);
        let title_style = context.world().node_style(parts.title).unwrap();
        assert!(title_style.layout.text_wraps());
        assert_eq!(
            title_style.layout.line_height,
            Some(LineHeightSpec::Absolute(line))
        );
    }

    #[test]
    fn action_and_close_stay_centred_as_copy_and_progress_change() {
        for theme in [
            nana_ui_core::ThemeDefinition::NANA_LIGHT,
            nana_ui_core::ThemeDefinition::NANA_DARK,
        ] {
            let mut context = AppContext::new();
            context
                .set_theme_definition(&theme.with_toast(ToastRecipe {
                    title_max_lines: 2,
                    description_max_lines: 3,
                    ..ToastRecipe::DEFAULT
                }))
                .unwrap();
            let toast = assembled(
                &mut context,
                Toast::new("Saved", ToastTone::Success)
                    .action("View")
                    .dismissible(true),
            );
            let parts = parts_of(&context, toast);
            for description in [
                None,
                Some(
                    "A long exported file path with several folders and a file name that wraps across lines",
                ),
            ] {
                for progress in [
                    None,
                    Some(ToastProgress::Determinate(0.5)),
                    Some(ToastProgress::Indeterminate),
                ] {
                    context
                        .update_component(toast, |toast, _| {
                            toast.description = description.map(Arc::from);
                            toast.progress = progress;
                        })
                        .unwrap();
                    for width in [220.0, 360.0] {
                        lay_out(&mut context, width);
                        let bounds = context.world().layout_box(toast.stable_id()).unwrap();
                        for button in [parts.action, parts.close] {
                            let button = context.world().layout_box(button).unwrap();
                            assert!(
                                ((button.y + button.height / 2.0)
                                    - (bounds.y + bounds.height / 2.0))
                                    .abs()
                                    < 0.01,
                                "button {button:?} is centred in {bounds:?}"
                            );
                        }
                    }
                }
            }
            assert_eq!(
                parts_of(&context, toast),
                parts,
                "updates retain the original controls"
            );
        }
    }

    /// A toast is as wide as its copy needs; one showing progress is at
    /// least the recipe's `progress.min_width`, so the bar does not jump.
    #[test]
    fn a_toast_showing_progress_is_at_least_the_recipe_min_width() {
        let mut context = AppContext::new();
        let plain = assembled(&mut context, Toast::new("Saved", ToastTone::Info));
        lay_out(&mut context, 800.0);
        let width = context.world().layout_box(plain.stable_id()).unwrap().width;
        let min = ToastRecipe::DEFAULT.progress.min_width;
        assert!(width < min, "fits its copy: {width}");

        context
            .update_component(plain, |toast, _| {
                toast.progress = Some(ToastProgress::Determinate(0.5));
            })
            .unwrap();
        lay_out(&mut context, 800.0);
        let width = context.world().layout_box(plain.stable_id()).unwrap().width;
        assert!((width - min).abs() < 0.01, "{width}");
        // The bar runs the width of the copy above it.
        let parts = parts_of(&context, plain);
        let copy = context.world().layout_box(parts.copy).unwrap();
        let bar = context.world().layout_box(parts.progress).unwrap();
        assert!(copy.width > 0.0);
        assert!((bar.width - copy.width).abs() < 0.01, "{bar:?} {copy:?}");
        assert!((bar.x - copy.x).abs() < 0.01);

        // A placed toast keeps the width it was given.
        context
            .update_component(plain, |toast, _| {
                toast.place_in(
                    LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: 800.0,
                        height: 600.0,
                    },
                    PopoverAlignment::End,
                    320.0,
                    PanelInsets::default(),
                );
            })
            .unwrap();
        lay_out(&mut context, 800.0);
        let width = context.world().layout_box(plain.stable_id()).unwrap().width;
        assert!((width - 320.0).abs() < 0.01, "{width}");
    }

    /// Without `assemble_toast` a toast stays the painted leaf it always was:
    /// one node with its title as text and painted regions, whatever its
    /// progress, action or icon say.
    #[test]
    fn a_bare_toast_keeps_its_painted_leaf_form() {
        let mut context = AppContext::new();
        let toast = context
            .create_component(
                document(),
                Toast::new("Saved", ToastTone::Success)
                    .description("Project exported")
                    .progress(ToastProgress::Determinate(0.5))
                    .action("Open")
                    .icon(Icon::Sparkles)
                    .dismissible(true),
            )
            .unwrap();
        let id = toast.stable_id();
        lay_out(&mut context, 800.0);
        assert!(children(&context, id).is_empty());
        assert_eq!(context.world().text(id), Some("Saved"));
        assert!(matches!(
            context.world().standard_visual(id),
            Some(crate::StandardVisual::Toast {
                assembled: false,
                icon: None,
                ..
            })
        ));
        assert!(matches!(
            context.world().component_geometry(id),
            Some(crate::ComponentGeometry::Toast {
                title: Some(_),
                description: Some(_),
                dismiss: Some(_),
                ..
            })
        ));
        assert_eq!(context.world().live(id), None);
        assert_eq!(
            context.world().node_style(id).unwrap().layout.width,
            Some(LengthSpec::Fill)
        );
        assert!(context.read(toast, |toast| toast.parts.is_none()).unwrap());
    }

    /// Writing a whole new toast over an assembled one keeps it assembled:
    /// its parts are the framework's, not the value's.
    #[test]
    fn writing_a_whole_toast_keeps_it_assembled() {
        let mut context = AppContext::new();
        let toast = assembled(&mut context, Toast::new("Optimizing", ToastTone::Info));
        let parts = parts_of(&context, toast);
        context
            .update_component(toast, |toast, _| {
                *toast = Toast::new("Optimized", ToastTone::Success)
                    .progress(ToastProgress::Determinate(1.0));
            })
            .unwrap();
        assert_eq!(parts_of(&context, toast), parts);
        assert_eq!(text_of(&context, parts.title), "Optimized");
        assert!(!hidden(&context, parts.progress));
        assert_eq!(context.world().text(toast.stable_id()), Some(""));
    }

    /// An assembled toast's timer holds while a pointer is over it or focus
    /// is inside it, and goes on with the time it had left.
    #[test]
    fn a_held_timer_keeps_the_time_it_had_left() {
        use std::time::Duration;
        let seconds = Duration::from_secs;
        let mut context = AppContext::new();
        let toast = assembled(
            &mut context,
            Toast::new("Saved", ToastTone::Success)
                .timeout(seconds(4))
                .dismissible(true),
        );
        let id = toast.stable_id();
        let parts = parts_of(&context, toast);
        lay_out(&mut context, 800.0);
        let dismissed = Arc::new(Mutex::new(0));
        let count = Arc::clone(&dismissed);
        context
            .on(toast, move |_, _: &ToastDismissed, _| {
                *count.lock().unwrap() += 1;
            })
            .unwrap();
        let dismissals = |dismissed: &Arc<Mutex<i32>>| *dismissed.lock().unwrap();

        // Counting from 10s; a second in, the pointer comes over it.
        context.advance_animations(seconds(10));
        context.advance_animations(seconds(11));
        context
            .set_pointer_hover_at(document(), 1, Some(id), seconds(11))
            .unwrap();
        context.advance_animations(seconds(20));
        assert_eq!(dismissals(&dismissed), 0, "held while hovered");
        assert_eq!(
            context.next_animation_deadline(),
            None,
            "nothing to wake for"
        );

        // It leaves at 20s: the three seconds left count from the frame that
        // wakes for them.
        let frame = Duration::from_millis(16);
        context
            .set_pointer_hover_at(document(), 1, None, seconds(20))
            .unwrap();
        assert!(
            context.next_animation_deadline().is_some(),
            "it wakes again"
        );
        context.advance_animations(seconds(20) + frame);
        context.advance_animations(seconds(22) + frame);
        assert_eq!(dismissals(&dismissed), 0);

        // Focus on its close button holds it too, a second before the end.
        context.focus_node(document(), parts.close).unwrap();
        context.advance_animations(seconds(30));
        assert_eq!(dismissals(&dismissed), 0, "held while focus is inside");
        context.clear_focus(document()).unwrap();
        context.advance_animations(seconds(30) + frame);
        context.advance_animations(seconds(31));
        assert_eq!(dismissals(&dismissed), 0, "a second was left");
        context.advance_animations(seconds(31) + frame);
        assert_eq!(dismissals(&dismissed), 1);
        context.advance_animations(seconds(40));
        assert_eq!(dismissals(&dismissed), 1, "once");
    }
}
