use std::sync::Arc;

#[cfg(test)]
use nana_ui_core::UI_METRICS;
use nana_ui_core::{
    FlexDirection, Icon, LengthSpec, OverflowSpec, PopoverAlignment, PopoverPlacement,
    PositionSpec, SemanticColorRole, StyleModelRef, UI_BASE_TEXT_SIZE,
};

use crate::view_components::project_common;
use crate::widget_visual::push_component_image;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, CONTEXT_MENU_HINTS,
    CONTEXT_MENU_IMAGE_BACKDROPS, CONTEXT_MENU_IMAGES, ComponentElevation, ComponentTextRegion,
    ComponentTriggerSurface, ComputedStyle, InteractionState, LayoutBox, MenuSurfaceKind,
    MutationQueue, NodeKind, NodeStyle, SelectOptionData, SelectOptionGeometry, StableNodeId,
    TRIGGERED_OVERLAY_SURFACE_SLOT, TextHorizontalAlignment, TextVerticalAlignment,
    TriggeredMenuOverlay, UiWorld, WidgetFloatingOverlay, WidgetFloatingOverlayKind, WidgetGlyph,
    WidgetIcon, WidgetPaintCtx, WidgetPaintOp, WidgetPaintOutput, WidgetQuad, WidgetTaggedOp,
    WidgetText, WidgetVisual, widget_collection_slot,
};

pub(crate) const POPOVER_WIDTH: f32 = 240.0;
const POPOVER_PADDING: f32 = nana_ui_core::space::LG;
const POPOVER_GAP: f32 = nana_ui_core::space::SM;
pub(crate) const ACTION_MENU_WIDTH: f32 = 200.0;
const ACTION_MENU_GAP: f32 = nana_ui_core::space::XS;
pub(crate) const MENU_MIN_WIDTH: f32 = 120.0;
#[cfg(test)]
const ACTION_MENU_PADDING: f32 = nana_ui_core::space::XS;
/// Indentation per tree level, shared by `TreeView` and sidebar tree rows.
pub(crate) const TREE_DEPTH_STEP: f32 = nana_ui_core::space::XL;
/// The `z-index` an open Popover, ActionMenu or HoverCard stacks at: its
/// content is viewport-fixed at this level in the root stacking context, and
/// its surface paints and takes the pointer there too, above the page.
pub const MENU_OVERLAY_Z_INDEX: i32 = 1_000;
/// Paint z-index base for floating overlays mounted at [`nana_ui_core::OverlayMount::TopLayer`].
pub const TOP_LAYER_Z_INDEX: i32 = 100_000;
/// The trigger is a real button, so it matches the compact control height
/// rather than hugging its glyphs.
#[cfg(test)]
pub(crate) const TRIGGER_HEIGHT: f32 = UI_METRICS.compact_control_height;
/// Icon triggers draw the standard control glyph size, centered in the chrome.
pub(crate) const TRIGGER_ICON_SIZE: f32 = UI_BASE_TEXT_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverToggled {
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverClosed;

/// In-flow trigger with an optional surface painted below it.
///
/// The trigger is the popover's own node: it takes the press, the focus and
/// Enter / Space, and the surface hangs off it. It shows a label
/// ([`Popover::trigger`]), a glyph ([`Popover::trigger_icon`]), or content of
/// the application's ([`Popover::trigger_content`], a view's `.trigger(..)`
/// slot): a row the trigger lays out inside its chrome — an icon, a label and
/// a count — while the popover stays the control.
#[derive(Debug, Clone, PartialEq)]
pub struct Popover {
    /// Text trigger label, or the accessible name when [`Popover::trigger_icon`]
    /// or [`Popover::trigger_content`] draws the trigger instead.
    pub trigger: Arc<str>,
    /// Spoken name when [`Self::trigger`] is too short. Empty keeps the trigger.
    pub accessible_name: Arc<str>,
    pub trigger_icon: Option<Icon>,
    /// A child that draws the trigger. It stays shown while the surface is
    /// closed and is not one of the surface's items; it is display only (the
    /// press goes to the popover), so it holds no control of its own.
    pub trigger_content: Option<StableNodeId>,
    pub open: bool,
    pub placement: PopoverPlacement,
    pub alignment: PopoverAlignment,
    pub gap: f32,
    pub width: f32,
    pub padding: f32,
    /// Standard default spacing; numeric setters clear the binding.
    pub gap_theme: Option<nana_ui_core::SpacingStep>,
    pub padding_theme: Option<nana_ui_core::SpacingStep>,
    pub close_on_escape: bool,
    pub close_on_outside: bool,
    /// Resting trigger has no fill and no border. Hover, press and the open
    /// state still wash the trigger so the hit target stays visible. Off by
    /// default: a trigger keeps the raised menu-button chrome.
    pub bare_trigger: bool,
    /// Use the shared 24px target when this popover has an icon trigger.
    pub compact_trigger: bool,
    /// An icon trigger's glyph size in logical pixels; `None` keeps the
    /// standard glyph. See [`Self::trigger_icon_size`].
    pub trigger_icon_size: Option<f32>,
    /// What the application writes over the trigger's own style while the
    /// popover has a trigger. Each field it sets replaces the recipe's and
    /// the rest stay the recipe's: the hover and press washes, the open
    /// state's, a compact trigger's fit. A radius it writes is the
    /// trigger's corners. See [`Self::trigger_style`].
    pub trigger_style: Option<NodeStyle>,
    /// Paint the trigger as this [`nana_ui_core::ButtonKind`] of the theme's
    /// button recipe (resting fill, ink, hover and press) instead of the menu
    /// button chrome; `None` keeps the chrome. See [`Self::trigger_tool`].
    pub trigger_kind: Option<nana_ui_core::ButtonKind>,
    /// Show [`Self::trigger`] beside [`Self::trigger_icon`] instead of only
    /// speaking it. See [`Self::trigger_tool`].
    pub trigger_labelled: bool,
    /// Visual mounting scope for the floating popover surface and its items
    /// (defaults to [`nana_ui_core::OverlayMount::NearestModal`]). The in-flow
    /// trigger always remains in its ancestor chain.
    pub overlay_mount: nana_ui_core::OverlayMount,
}

impl Popover {
    pub fn new() -> Self {
        Self {
            trigger: Arc::from(""),
            accessible_name: Arc::from(""),
            trigger_icon: None,
            trigger_content: None,
            open: false,
            placement: PopoverPlacement::Bottom,
            alignment: PopoverAlignment::Center,
            gap: POPOVER_GAP,
            width: POPOVER_WIDTH,
            padding: POPOVER_PADDING,
            gap_theme: Some(nana_ui_core::SpacingStep::Sm),
            padding_theme: Some(nana_ui_core::SpacingStep::Lg),
            close_on_escape: true,
            close_on_outside: true,
            bare_trigger: false,
            compact_trigger: false,
            trigger_icon_size: None,
            trigger_style: None,
            trigger_kind: None,
            trigger_labelled: false,
            overlay_mount: nana_ui_core::OverlayMount::NearestModal,
        }
    }

    pub fn trigger(mut self, trigger: impl Into<Arc<str>>) -> Self {
        self.trigger = trigger.into();
        self
    }

    /// Style the trigger: `style`'s fields are written over the trigger's
    /// recipe, which keeps everything `style` leaves unset — set only a
    /// width, and the trigger still washes on hover and press. The hanging
    /// surface keeps [`Self::width`] and [`Self::padding`].
    pub fn trigger_style(mut self, style: NodeStyle) -> Self {
        self.trigger_style = Some(style);
        self
    }

    /// Speak `name` while the trigger still paints [`Self::trigger`].
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.accessible_name = name.into();
        self
    }

    /// Icon trigger with an accessible name; the glyph is drawn centered in a
    /// square chrome instead of riding the label's text metrics.
    pub fn trigger_icon(mut self, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        self.trigger = label.into();
        self.trigger_icon = Some(icon);
        self
    }

    /// Draw the trigger with the child `content` (already a child of the
    /// popover, or placed by a view's `.trigger(..)` slot); `trigger` stays
    /// its accessible name.
    pub fn trigger_content(mut self, content: StableNodeId) -> Self {
        self.trigger_content = Some(content);
        self
    }

    /// Draw an icon trigger's glyph `size` logical pixels square, so it can
    /// match the [`crate::IconButton::icon_size`] of the tools beside it. The
    /// trigger's box is unchanged. A size that is not finite or below zero is
    /// ignored.
    pub fn trigger_icon_size(mut self, size: f32) -> Self {
        if size.is_finite() && size >= 0.0 {
            self.trigger_icon_size = Some(size);
        }
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn alignment(mut self, alignment: PopoverAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self.gap_theme = None;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(MENU_MIN_WIDTH);
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding.max(0.0);
        self.padding_theme = None;
        self
    }

    pub fn close_on_escape(mut self, enabled: bool) -> Self {
        self.close_on_escape = enabled;
        self
    }

    pub fn close_on_outside(mut self, enabled: bool) -> Self {
        self.close_on_outside = enabled;
        self
    }

    /// Drop the trigger's resting fill and border. Hover, press and open still
    /// wash it. The hanging surface is unchanged.
    pub fn bare_trigger(mut self, bare: bool) -> Self {
        self.bare_trigger = bare;
        self
    }

    /// Paint the trigger as `kind` of the button recipe; see
    /// [`Self::trigger_kind`](field@Self::trigger_kind).
    pub fn trigger_kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.trigger_kind = Some(kind);
        self
    }

    /// A heading's labelled tool that opens this menu: `icon` and `label`
    /// side by side in a [`crate::Button::tool`]'s box, painted as `kind`.
    pub fn trigger_tool(
        mut self,
        icon: Icon,
        label: impl Into<Arc<str>>,
        kind: nana_ui_core::ButtonKind,
    ) -> Self {
        self.trigger = label.into();
        self.trigger_icon = Some(icon);
        self.trigger_kind = Some(kind);
        self.trigger_labelled = true;
        self.compact_trigger = true;
        self
    }

    /// Fit an icon trigger into a 24×24 logical-pixel heading tool. A
    /// custom-content trigger instead takes exactly its content's box, with
    /// no inset and no fixed height. Text triggers retain their regular
    /// layout.
    pub fn compact_trigger(mut self, compact: bool) -> Self {
        self.compact_trigger = compact;
        self
    }

    /// Set where the floating popover surface mounts visually in the scene tree
    /// (defaults to [`nana_ui_core::OverlayMount::NearestModal`]).
    pub fn overlay_mount(mut self, mount: nana_ui_core::OverlayMount) -> Self {
        self.overlay_mount = mount;
        self
    }
}

impl Default for Popover {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) static POPOVER_CAPABILITIES: crate::framework::PopoverCapabilities =
    crate::framework::PopoverCapabilities {
        dismiss_on_escape: AppContext::dismiss_popovers_on_escape,
        close_within: AppContext::close_open_popovers_within,
        dismiss_outside: AppContext::dismiss_popovers_outside,
    };

fn install_popover_capabilities(caps: &mut crate::framework::Capabilities) {
    caps.popover = Some(&POPOVER_CAPABILITIES);
    caps.add_open_menu_navigator(|cx, doc, key, _| cx.navigate_open_action_menu(doc, key));
}

impl crate::ComponentView for Popover {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::POPOVER.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(install_popover_capabilities),
        activation: Some(crate::AppContext::toggle_popover),
        activate_at: Some(crate::AppContext::activate_popover_at),
        lifecycle: Some(crate::AppContext::settle_popover),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "popover".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_MENU
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_BUTTON)
    }

    /// Items attach and detach under the open surface, so the anchored
    /// projection must re-run with them to keep the origin math current.
    fn wants_child_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::Popover,
            Some(Arc::clone(&self.trigger)).filter(|value| !value.is_empty()),
            self.trigger_icon,
            self.trigger_icon_size,
            self.trigger_content,
            self.open,
            self.width,
            self.padding_theme.map_or(self.padding, |step| {
                world.node_theme(id).spacing().resolve(step)
            }),
            self.gap_theme.map_or(self.gap, |step| {
                world.node_theme(id).spacing().resolve(step)
            }),
            self.placement,
            self.alignment,
            "popover",
            self.bare_trigger,
            self.compact_trigger,
            &self.accessible_name,
            self.trigger_style.as_ref(),
            TriggerTool {
                kind: self.trigger_kind,
                labelled: self.trigger_labelled,
            },
            false,
            self.overlay_mount,
        );
    }
}

/// Trigger-bound action menu. Same surface as Popover with start alignment.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionMenu {
    pub popover: Popover,
    pub disabled: bool,
}

impl ActionMenu {
    pub fn new() -> Self {
        let default_padding = nana_ui_core::MenuRecipe::DEFAULT.surface_padding;
        let mut popover = Popover::new()
            .alignment(PopoverAlignment::Start)
            .gap(ACTION_MENU_GAP)
            .width(ACTION_MENU_WIDTH)
            .padding(nana_ui_core::SpacingTokens::DEFAULT.resolve(default_padding));
        popover.gap_theme = Some(nana_ui_core::SpacingStep::Xs);
        popover.padding_theme = Some(default_padding);
        Self {
            popover,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self.popover.open &= !disabled;
        self
    }

    pub fn trigger(mut self, trigger: impl Into<Arc<str>>) -> Self {
        self.popover = self.popover.trigger(trigger);
        self
    }

    /// Speak `name` while the trigger still paints its visible label.
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.popover = self.popover.accessible_name(name);
        self
    }

    pub fn trigger_icon(mut self, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        self.popover = self.popover.trigger_icon(icon, label);
        self
    }

    /// The icon trigger's glyph size; see [`Popover::trigger_icon_size`].
    pub fn trigger_icon_size(mut self, size: f32) -> Self {
        self.popover = self.popover.trigger_icon_size(size);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.popover.open = open && !self.disabled;
        self
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.popover = self.popover.placement(placement);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.popover = self.popover.width(width);
        self
    }

    pub fn bare_trigger(mut self, bare: bool) -> Self {
        self.popover = self.popover.bare_trigger(bare);
        self
    }

    /// Use the same compact icon target as [`crate::IconButton::compact`].
    pub fn compact_trigger(mut self, compact: bool) -> Self {
        self.popover = self.popover.compact_trigger(compact);
        self
    }

    /// Style the trigger over its recipe; see [`Popover::trigger_style`].
    pub fn trigger_style(mut self, style: NodeStyle) -> Self {
        self.popover = self.popover.trigger_style(style);
        self
    }

    /// Paint the trigger as `kind`; see [`Popover::trigger_kind`].
    pub fn trigger_kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.popover = self.popover.trigger_kind(kind);
        self
    }

    /// A heading's labelled tool; see [`Popover::trigger_tool`].
    pub fn trigger_tool(
        mut self,
        icon: Icon,
        label: impl Into<Arc<str>>,
        kind: nana_ui_core::ButtonKind,
    ) -> Self {
        self.popover = self.popover.trigger_tool(icon, label, kind);
        self
    }

    /// Set where the floating menu surface mounts visually in the scene tree
    /// (defaults to [`nana_ui_core::OverlayMount::NearestModal`]).
    pub fn overlay_mount(mut self, mount: nana_ui_core::OverlayMount) -> Self {
        self.popover = self.popover.overlay_mount(mount);
        self
    }
}

impl Default for ActionMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::ComponentView for ActionMenu {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::ACTION_MENU.type_id);

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_MENU
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_BUTTON)
    }

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(install_popover_capabilities),
        activation: Some(crate::AppContext::toggle_action_menu),
        activate_at: Some(crate::AppContext::activate_action_menu_at),
        lifecycle: Some(crate::AppContext::settle_action_menu),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "action-menu".into(),
        }
    }

    /// Items attach and detach under the open surface, so the anchored
    /// projection must re-run with them to keep the origin math current.
    fn wants_child_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let node_theme = world.node_theme(id);
        let padding = self
            .popover
            .padding_theme
            .map_or(self.popover.padding, |_| {
                node_theme
                    .spacing()
                    .resolve(node_theme.recipes().menu().surface_padding)
            });
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::ActionMenu,
            Some(Arc::clone(&self.popover.trigger)).filter(|value| !value.is_empty()),
            self.popover.trigger_icon,
            self.popover.trigger_icon_size,
            self.popover.trigger_content,
            self.popover.open,
            self.popover.width,
            padding,
            self.popover
                .gap_theme
                .map_or(self.popover.gap, |step| node_theme.spacing().resolve(step)),
            self.popover.placement,
            self.popover.alignment,
            "action-menu",
            self.popover.bare_trigger,
            self.popover.compact_trigger,
            &self.popover.accessible_name,
            self.popover.trigger_style.as_ref(),
            TriggerTool {
                kind: self.popover.trigger_kind,
                labelled: self.popover.trigger_labelled,
            },
            self.disabled,
            self.popover.overlay_mount,
        );
    }
}

/// How a trigger stands in as a heading's tool: its button kind and whether
/// it shows its label beside its glyph.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TriggerTool {
    pub(crate) kind: Option<nana_ui_core::ButtonKind>,
    pub(crate) labelled: bool,
}

pub(crate) fn project_menu_surface(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    kind: MenuSurfaceKind,
    trigger: Option<Arc<str>>,
    trigger_icon: Option<Icon>,
    trigger_icon_size: Option<f32>,
    trigger_content: Option<StableNodeId>,
    open: bool,
    width: f32,
    padding: f32,
    gap: f32,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    label: &str,
    bare_trigger: bool,
    compact_trigger: bool,
    accessible_name: &Arc<str>,
    trigger_style: Option<&NodeStyle>,
    tool: TriggerTool,
    disabled: bool,
    overlay_mount: nana_ui_core::OverlayMount,
) {
    let open = world.project_menu_presence(id, open && !disabled, mutations);
    // A child that draws the trigger only counts while it is one.
    let trigger_content = trigger_content.filter(|content| world.parent_id(*content) == Some(id));
    let has_trigger = trigger.is_some() || trigger_icon.is_some() || trigger_content.is_some();
    let desired_mount = (!has_trigger && open).then_some(overlay_mount);
    if world.overlay_mount(id) != desired_mount {
        mutations.set_overlay_mount(id, desired_mount);
    }
    let labelled = tool.labelled
        && trigger_icon.is_some()
        && trigger_content.is_none()
        && trigger.as_deref().is_some_and(|label| !label.is_empty());
    // A labelled tool's glyph is an icon tool's, at the medium step, unless
    // set: at the small step its strokes read paler than the label.
    let node_theme = world.node_theme(id);
    let trigger_icon_size = if labelled {
        Some(trigger_icon_size.unwrap_or_else(|| {
            nana_ui_core::ControlSize::Medium.text_size_in(node_theme.typography())
        }))
    } else {
        trigger_icon_size
    };
    let labelled_gap = labelled.then(|| node_theme.spacing().sm);
    let has_chrome = open || has_trigger;
    if has_chrome {
        let visual = MenuSurfaceVisual {
            kind,
            open,
            // The label is the accessible name only while content draws it.
            trigger: trigger.clone().filter(|_| trigger_content.is_none()),
            trigger_icon,
            trigger_image: None,
            gap,
            overlay: has_trigger.then_some(TriggeredMenuOverlay {
                placement,
                alignment,
                width: width.max(MENU_MIN_WIDTH),
                padding,
                gap,
                trigger_content,
                trigger_icon_size: trigger_icon_size.filter(|_| trigger_icon.is_some()),
                trigger_label_gap: labelled_gap,
                overlay_mount,
            }),
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        };
        if world.widget_visual_as::<MenuSurfaceVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
    } else if world.widget_visual(id).is_some() {
        mutations.set_widget_visual::<MenuSurfaceVisual>(id, None);
    }
    // The trigger label is measured like any other button label, so the closed
    // surface can size itself to the text instead of a fixed box. Icon
    // triggers keep their label as the accessible name only; the square chrome
    // comes from the style instead of text measurement.
    let trigger_text = if (trigger_icon.is_some() && !labelled) || trigger_content.is_some() {
        ""
    } else {
        trigger.as_deref().unwrap_or("")
    };
    if world.text(id) != Some(trigger_text) {
        mutations.set_text(
            id,
            crate::TextContent {
                value: trigger_text.to_string().into(),
            },
        );
    }
    let style = if trigger_content.is_some() {
        trigger_content_style()
    } else {
        triggered_menu_style(
            node_theme,
            trigger_icon.is_some(),
            trigger.as_deref(),
            open,
            width,
            padding,
        )
    };
    let style = if let (true, Some(gap), Some(glyph)) =
        (compact_trigger, labelled_gap, trigger_icon_size)
    {
        // The label is measured as text; the glyph and its gap ride in the
        // leading inset.
        let mut style = crate::view_components::tool_box(trigger_button_style());
        let inset = node_theme.metrics().small_control_padding_x();
        let layout = Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(LengthSpec::Px(inset + glyph + gap));
        layout.padding_right = Some(LengthSpec::Px(inset));
        style.control_padding_x = None;
        style
    } else if labelled {
        style
    } else if compact_trigger && trigger_icon.is_some() && trigger_content.is_none() {
        crate::view_components::compact_icon_style(style)
    } else if compact_trigger && trigger_content.is_some() {
        fitted_content_style(style)
    } else {
        style
    };
    let style = if bare_trigger && has_trigger {
        bare_trigger_style(style, open)
    } else {
        style
    };
    let style = match tool.kind.filter(|_| has_trigger) {
        Some(kind) => {
            let variant = node_theme.recipes().button().variant(kind);
            let mut style = style;
            style.foreground = Some(variant.foreground);
            style.background = if open {
                Some(variant.pressed_background)
            } else {
                variant.background
            };
            style.border = variant.border;
            if variant.border.is_none() {
                Arc::make_mut(&mut style.layout).border_width = Some(0.0);
            }
            style.interaction.hovered.background = Some(variant.hovered_background);
            style.interaction.pressed.background = Some(variant.pressed_background);
            style
        }
        None => style,
    };
    let style = match trigger_style.filter(|_| has_trigger) {
        Some(over) => style.overlaid(over),
        None => style,
    };
    project_common(
        id,
        world,
        mutations,
        &style,
        InteractionState {
            pointer_events: !disabled && (open || has_trigger),
            focusable: !disabled && has_trigger,
        },
        AccessibilityState {
            role: AccessibilityRole::Menu,
            disabled,
            label: Some(if accessible_name.is_empty() {
                trigger.unwrap_or_else(|| Arc::from(label))
            } else {
                Arc::clone(accessible_name)
            }),
            ..AccessibilityState::default()
        },
    );
}

/// The surface an open triggered menu (Popover, ActionMenu, HoverCard) hangs
/// above the page, in viewport coordinates: its items' box and padding.
/// `None` while closed, and for a menu with no trigger, whose surface is its
/// own in-flow box.
pub(crate) fn hanging_surface(world: &UiWorld, id: StableNodeId) -> Option<LayoutBox> {
    let MenuSurfaceVisual {
        open: true,
        overlay: Some(overlay),
        ..
    } = world.widget_visual_as::<MenuSurfaceVisual>(id)?
    else {
        return None;
    };
    let visual = world.widget_visual_as::<MenuSurfaceVisual>(id)?;
    let has_trigger = visual.has_trigger();
    let surface = overlay_surface_from_items(world, id, Some(overlay));
    (has_trigger && surface.width > 0.0 && surface.height > 0.0).then_some(surface)
}

pub(crate) fn overlay_surface_from_items(
    world: &UiWorld,
    id: StableNodeId,
    overlay: Option<&TriggeredMenuOverlay>,
) -> LayoutBox {
    let Some(overlay) = overlay else {
        return LayoutBox::default();
    };
    let children = world.node(id).map(|node| node.children).unwrap_or_default();
    let children = children
        .into_iter()
        .filter(|child| overlay.trigger_content != Some(*child));
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut any = false;
    for child in children {
        // A closed branch leaves the items' last boxes behind until layout
        // writes zeros. Those boxes are not on screen; counting them paints
        // the card after it has already closed.
        if !world.is_overlay_reachable(child) {
            continue;
        }
        let Some(child_box) = world.component_layout_box(child) else {
            continue;
        };
        if child_box.width <= 0.0 && child_box.height <= 0.0 {
            continue;
        }
        any = true;
        min_x = min_x.min(child_box.x);
        min_y = min_y.min(child_box.y);
        max_x = max_x.max(child_box.x + child_box.width);
        max_y = max_y.max(child_box.y + child_box.height);
    }
    if !any {
        return LayoutBox::default();
    }
    LayoutBox {
        x: min_x - overlay.padding,
        y: min_y - overlay.padding,
        width: (max_x - min_x) + overlay.padding * 2.0,
        height: (max_y - min_y) + overlay.padding * 2.0,
    }
}

fn triggered_menu_style(
    node_theme: &nana_ui_core::CompiledTheme,
    icon_trigger: bool,
    trigger: Option<&str>,
    open: bool,
    width: f32,
    padding: f32,
) -> NodeStyle {
    if icon_trigger {
        return trigger_icon_button_style();
    }
    if trigger.is_some() {
        return trigger_button_style();
    }
    let menu = node_theme.recipes().menu();
    if !open {
        // Nothing to press and nothing to show, so the node keeps the smallest
        // box that stays out of the way.
        return NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                width: Some(LengthSpec::Px(nana_ui_core::HAIRLINE)),
                height: Some(LengthSpec::Px(nana_ui_core::HAIRLINE)),
                overflow_x: OverflowSpec::Hidden,
                overflow_y: OverflowSpec::Hidden,
                ..nana_ui_core::LayoutStyle::default()
            }),
            foreground: Some(menu.item_foreground),
            ..NodeStyle::default()
        };
    }
    let item_gap = node_theme.spacing().resolve(menu.item_gap);
    let mut theme = nana_ui_core::ThemeLayout::default();
    theme.gap = Some(nana_ui_core::ThemeToken::MenuItemGap.into());
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Px(width.max(MENU_MIN_WIDTH))),
            min_width: Some(LengthSpec::Px(MENU_MIN_WIDTH)),
            direction: Some(FlexDirection::Column),
            gap: Some(LengthSpec::Px(item_gap)),
            theme: theme.into(),
            padding_left: Some(LengthSpec::Px(padding)),
            padding_right: Some(LengthSpec::Px(padding)),
            padding_top: Some(LengthSpec::Px(padding)),
            padding_bottom: Some(LengthSpec::Px(padding)),
            ..nana_ui_core::LayoutStyle::default()
        }),
        foreground: Some(menu.item_foreground),
        radius: Some(menu.surface_radius),
        ..NodeStyle::default()
    }
}

/// The trigger carries the same chrome contract as [`ButtonKind::Menu`]
/// (`Button`), so a standalone menu button and an in-place action-menu trigger
/// read as one control; hover and press colours resolve through the usual
/// interaction overlay.
pub(crate) fn trigger_button_style() -> NodeStyle {
    let mut style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            // Keep the trigger content-sized in a stretching column without
            // overriding the parent's cross-axis alignment in a toolbar row.
            width: Some(LengthSpec::Shrink),
            border_width: Some(nana_ui_core::HAIRLINE),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Subtle),
        border: Some(SemanticColorRole::BorderSoft),
        foreground: Some(SemanticColorRole::Text),
        shape: Some(nana_ui_core::ShapeRole::Button),
        control_height: Some(nana_ui_core::ControlHeight::Exact(
            nana_ui_core::ControlSize::Small,
        )),
        control_padding_x: Some(nana_ui_core::ControlPadding::Standard),
        ..NodeStyle::default()
    };
    style.interaction.hovered.background = Some(SemanticColorRole::Hover);
    style.interaction.pressed.background = Some(SemanticColorRole::Active);
    style
}

/// A trigger drawn by a child shares the text trigger's chrome and lays the
/// child out inside it, centred on the cross axis.
pub(crate) fn trigger_content_style() -> NodeStyle {
    let mut style = trigger_button_style();
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = nana_ui_core::AlignSpec::Center;
    style
}

/// A compact content trigger is exactly its content: no inset and no fixed
/// height, so a child that is itself the target (a color well) fills it to
/// the edge. It keeps the control corners, so the wash follows a well
/// drawn with them.
fn fitted_content_style(mut style: NodeStyle) -> NodeStyle {
    style.control_height = None;
    style.control_padding_x = None;
    style
}

/// Icon triggers share the text trigger's chrome but take a square min box, so
/// the glyph centers geometrically instead of riding text metrics.
pub(crate) fn trigger_icon_button_style() -> NodeStyle {
    let mut style = trigger_button_style();
    style.square = Some(nana_ui_core::SquareSize::Control(
        nana_ui_core::ControlSize::Small,
    ));
    style
}

/// Resting trigger without a fill or a border. Hover and press keep the wash
/// already on `style`. While open, the same hover wash marks the anchor.
fn bare_trigger_style(mut style: NodeStyle, open: bool) -> NodeStyle {
    style.background = if open {
        Some(SemanticColorRole::Hover)
    } else {
        None
    };
    style.border = None;
    let layout = Arc::make_mut(&mut style.layout);
    layout.border_width = Some(0.0);
    style
}

pub(crate) fn menu_surface_style(width: f32, padding: f32) -> NodeStyle {
    menu_surface_style_in(
        width,
        padding,
        nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Dark),
    )
}

pub(crate) fn menu_surface_style_in(
    width: f32,
    padding: f32,
    compiled: &nana_ui_core::CompiledTheme,
) -> NodeStyle {
    let menu = compiled.recipes().menu();
    let mut theme = nana_ui_core::ThemeLayout::default();
    theme.gap = Some(nana_ui_core::ThemeToken::MenuItemGap.into());
    theme.padding_left = Some(nana_ui_core::ThemeToken::MenuSurfacePadding.into());
    theme.padding_right = Some(nana_ui_core::ThemeToken::MenuSurfacePadding.into());
    theme.padding_top = Some(nana_ui_core::ThemeToken::MenuSurfacePadding.into());
    theme.padding_bottom = Some(nana_ui_core::ThemeToken::MenuSurfacePadding.into());
    theme.border_width =
        Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
    theme.font_size = Some(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Body).into());
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: PositionSpec::Fixed,
            width: Some(LengthSpec::Px(width)),
            min_width: Some(LengthSpec::Px(MENU_MIN_WIDTH)),
            direction: Some(FlexDirection::Column),
            gap: Some(LengthSpec::Px(compiled.spacing().resolve(menu.item_gap))),
            theme: theme.into(),
            padding_left: Some(LengthSpec::Px(padding)),
            padding_right: Some(LengthSpec::Px(padding)),
            padding_top: Some(LengthSpec::Px(padding)),
            padding_bottom: Some(LengthSpec::Px(padding)),
            border_width: Some(nana_ui_core::HAIRLINE),
            z_index: Some(MENU_OVERLAY_Z_INDEX),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(menu.surface_background),
        border: Some(menu.surface_border),
        foreground: Some(menu.item_foreground),
        radius: Some(menu.surface_radius),
        ..NodeStyle::default()
    }
}

pub(crate) fn apply_menu_surface_recipe(
    style: &mut NodeStyle,
    theme: &nana_ui_core::CompiledTheme,
) {
    let menu = theme.recipes().menu();
    style.background = Some(menu.surface_background);
    style.border = Some(menu.surface_border);
    style.foreground = Some(menu.item_foreground);
    style.radius = Some(menu.surface_radius);
    let pad = theme.spacing().resolve(menu.surface_padding);
    let layout = Arc::make_mut(&mut style.layout);
    layout.gap = Some(LengthSpec::Px(theme.spacing().resolve(menu.item_gap)));
    layout.padding_left = Some(LengthSpec::Px(pad));
    layout.padding_right = Some(LengthSpec::Px(pad));
    layout.padding_top = Some(LengthSpec::Px(pad));
    layout.padding_bottom = Some(LengthSpec::Px(pad));
}

pub(crate) fn project_anchored_menu(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    kind: MenuSurfaceKind,
    style: &NodeStyle,
    open: bool,
    overlay_mount: nana_ui_core::OverlayMount,
    label: &str,
) {
    let open = world.project_menu_presence(id, open, mutations);
    if world.overlay_mount(id) != Some(overlay_mount) {
        mutations.set_overlay_mount(id, Some(overlay_mount));
    }
    if open {
        let visual = MenuSurfaceVisual {
            kind,
            open,
            trigger: None,
            trigger_icon: None,
            trigger_image: None,
            gap: 0.0,
            overlay: None,
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        };
        if world.widget_visual_as::<MenuSurfaceVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
    } else if world.widget_visual(id).is_some() {
        mutations.set_widget_visual::<MenuSurfaceVisual>(id, None);
    }
    let mut style = style.clone();
    apply_menu_surface_recipe(&mut style, world.node_theme(id));
    Arc::make_mut(&mut style.layout).hidden = !open;
    project_common(
        id,
        world,
        mutations,
        &style,
        InteractionState {
            pointer_events: open,
            focusable: false,
        },
        AccessibilityState {
            role: AccessibilityRole::Menu,
            label: Some(Arc::from(label)),
            ..AccessibilityState::default()
        },
    );
}

/// Visual contract attached to triggered menus, action menus, hover cards,
/// and pointer-anchored context menus.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuSurfaceVisual {
    pub kind: MenuSurfaceKind,
    pub open: bool,
    pub trigger: Option<Arc<str>>,
    pub trigger_icon: Option<Icon>,
    /// Host-texture trigger (hover-card avatar). Painted by the node's own
    /// custom render slot; the label never becomes a text region.
    pub trigger_image: Option<Arc<str>>,
    pub gap: f32,
    /// Present when this node is an in-flow trigger; the menu items layout
    /// as a separate overlay column and must not share this node's box.
    pub overlay: Option<TriggeredMenuOverlay>,
    pub query: Option<Arc<str>>,
    pub rows: Arc<[SelectOptionData]>,
    pub highlighted: Option<usize>,
}

/// Projected geometry for a [`MenuSurfaceVisual`].
#[derive(Debug, Clone, PartialEq)]
pub struct MenuSurfaceGeometry {
    pub trigger: Option<ComponentTextRegion>,
    pub trigger_icon: Option<(Icon, LayoutBox)>,
    /// Image trigger (hover-card avatar): the chrome is circular and the
    /// texture rides the node's own custom render slot.
    pub trigger_image: Option<Arc<str>>,
    pub trigger_surface: Option<ComponentTriggerSurface>,
    pub surface: LayoutBox,
    pub search: Option<ComponentTextRegion>,
    pub search_field: Option<LayoutBox>,
    pub options: Vec<SelectOptionGeometry>,
    pub elevation: ComponentElevation,
    pub background: [f32; 4],
    pub border: [f32; 4],
}

impl MenuSurfaceGeometry {
    /// Indexed access to projected context-menu rows.
    pub fn option(&self, index: usize) -> Option<&SelectOptionGeometry> {
        self.options.get(index)
    }
}

impl MenuSurfaceVisual {
    pub(crate) fn has_trigger(&self) -> bool {
        self.trigger.is_some()
            || self.trigger_icon.is_some()
            || self.trigger_image.is_some()
            || self.overlay.is_some_and(|o| o.trigger_content.is_some())
    }

    pub(crate) fn geometry_for(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: LayoutBox,
        style: &ComputedStyle,
        style_model: StyleModelRef,
        theme: &nana_ui_core::CompiledTheme,
    ) -> MenuSurfaceGeometry {
        if self.kind == MenuSurfaceKind::ContextMenu
            && (self.query.is_some() || !self.rows.is_empty())
        {
            return crate::menus::context_menu_geometry_in(
                bounds,
                self.query.as_ref(),
                &self.rows,
                self.highlighted,
                &style_model.palette,
                style_model.metrics,
                &world.framework_strings().menu_search,
                world.chrome_text_measure(id),
                theme,
            );
        }
        let trigger_content = self.overlay.and_then(|overlay| overlay.trigger_content);
        let has_trigger = self.has_trigger();
        let surface = if has_trigger {
            if self.open {
                overlay_surface_from_items(world, id, self.overlay.as_ref())
            } else {
                LayoutBox::default()
            }
        } else {
            bounds
        };
        menu_surface_geometry(
            bounds,
            self.trigger.as_ref(),
            self.trigger_icon,
            self.trigger_image.as_ref(),
            trigger_content.is_some(),
            self.overlay
                .is_some_and(|overlay| overlay.trigger_label_gap.is_some()),
            style,
            style_model,
            theme,
            surface,
            &world.component_content_parts(id),
        )
    }
}

impl WidgetVisual for MenuSurfaceVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::PALETTE
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_MENU)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn is_menu_surface(&self) -> bool {
        true
    }

    fn geometry_reads_page(&self) -> bool {
        self.open
    }

    fn has_virtual_nodes(&self) -> bool {
        self.open && self.kind == MenuSurfaceKind::ContextMenu
    }

    fn is_action_menu_open(&self) -> bool {
        self.open && self.kind == MenuSurfaceKind::ActionMenu
    }

    fn closed_menu_hides(&self, child: StableNodeId) -> bool {
        !self.open && self.overlay.and_then(|overlay| overlay.trigger_content) != Some(child)
    }

    fn triggered_menu_overlay(&self) -> Option<TriggeredMenuOverlay> {
        self.open
            .then_some(self.overlay)
            .flatten()
            .filter(|_| self.has_trigger())
    }

    fn overlay_mount(&self) -> Option<nana_ui_core::OverlayMount> {
        self.overlay.map(|overlay| overlay.overlay_mount)
    }

    fn paint_depends_on_child(&self, child: StableNodeId) -> bool {
        self.open
            && self
                .overlay
                .is_some_and(|overlay| overlay.trigger_content != Some(child))
    }

    fn presentation_pivot(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: LayoutBox,
    ) -> Option<[f32; 2]> {
        if !self.open || !self.has_trigger() {
            return None;
        }
        let surface = overlay_surface_from_items(world, id, self.overlay.as_ref());
        let x = (bounds.x + bounds.width / 2.0)
            .min(surface.x + surface.width)
            .max(surface.x);
        let y = (bounds.y + bounds.height / 2.0)
            .min(surface.y + surface.height)
            .max(surface.y);
        Some([x - bounds.x, y - bounds.y])
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        true
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        prev.and_then(|prev| prev.downcast_ref::<Self>())
            .is_some_and(|old| {
                old.overlay.and_then(|overlay| overlay.trigger_icon_size)
                    != self.overlay.and_then(|overlay| overlay.trigger_icon_size)
            })
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: LayoutBox,
        _content: LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        use crate::layout_engine::parts::{NO_CONTENT_PARTS, content_parts};
        if let (Some(_), Some(gap)) = (
            self.trigger_icon,
            self.overlay.and_then(|overlay| overlay.trigger_label_gap),
        ) {
            let padding = world.used_layout_padding(id);
            let extent = self
                .overlay
                .and_then(|overlay| overlay.trigger_icon_size)
                .unwrap_or(TRIGGER_ICON_SIZE)
                .min(bounds.height);
            let lane = bounds.x + padding.left;
            return Some(content_parts([
                Some(crate::LayoutPart::new(
                    crate::LayoutPartKind::LeadingContent,
                    LayoutBox {
                        x: lane - gap - extent,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    },
                )),
                Some(crate::LayoutPart::new(
                    crate::LayoutPartKind::TextContent,
                    LayoutBox {
                        x: lane,
                        width: (bounds.width - padding.left - padding.right).max(0.0),
                        ..bounds
                    },
                )),
            ]));
        }
        if self.trigger_icon.is_some() {
            let extent = self
                .overlay
                .and_then(|overlay| overlay.trigger_icon_size)
                .unwrap_or(TRIGGER_ICON_SIZE)
                .min(bounds.width)
                .min(bounds.height);
            return Some(content_parts([Some(crate::LayoutPart::new(
                crate::LayoutPartKind::LeadingContent,
                LayoutBox {
                    x: bounds.x + (bounds.width - extent) / 2.0,
                    y: bounds.y + (bounds.height - extent) / 2.0,
                    width: extent,
                    height: extent,
                },
            ))]));
        }
        if self.trigger_image.is_none()
            && self.trigger.as_ref().is_some_and(|label| !label.is_empty())
        {
            let padding = world.used_layout_padding(id);
            return Some(content_parts([Some(crate::LayoutPart::new(
                crate::LayoutPartKind::TextContent,
                LayoutBox {
                    x: bounds.x + padding.left,
                    width: (bounds.width - padding.left - padding.right).max(0.0),
                    ..bounds
                },
            ))]));
        }
        Some(NO_CONTENT_PARTS)
    }

    fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _layout: LayoutBox,
        available_width: f32,
        _available_height: f32,
    ) -> Option<f32> {
        Some(
            world
                .component_content_box(id)
                .and_then(|_| {
                    crate::layout_engine::parts::part_bounds(
                        &world.component_content_parts(id),
                        crate::LayoutPartKind::TextContent,
                    )
                })
                .map_or(available_width, |part| part.width.max(0.0)),
        )
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let geometry = self.geometry_for(
            ctx.id,
            ctx.world,
            ctx.bounds,
            ctx.style,
            ctx.style_model,
            ctx.theme,
        );
        let mut out = self.paint_with_geometry(
            &geometry,
            ctx.source_style.layout.as_ref(),
            ctx.style,
            ctx.style_model,
            ctx.theme,
            ctx.chrome_radii,
        );
        // Keep identifying triggered menus once closed so the trigger (and any
        // `trigger_content` subtree) stays outside the `FillMode::Forwards`
        // closing fade/scale retained on the menu node (mirroring
        // `UiWorld::has_triggered_menu_chrome`).
        if let Some(overlay) = self.overlay.filter(|_| self.has_trigger()) {
            out.floating_overlay = Some(WidgetFloatingOverlay {
                kind: WidgetFloatingOverlayKind::TriggeredSurface,
                overlay_mount: overlay.overlay_mount,
                trigger_content: overlay.trigger_content,
            });
            out.presentation_pivot = self.presentation_pivot(ctx.id, ctx.world, ctx.bounds);
        }
        out
    }
}

impl MenuSurfaceVisual {
    pub fn paint_with_geometry(
        &self,
        geometry: &MenuSurfaceGeometry,
        layout: &nana_ui_core::LayoutStyle,
        style: &crate::ComputedStyle,
        style_model: StyleModelRef,
        theme: &nana_ui_core::CompiledTheme,
        chrome_radii: nana_ui_core::ChromeRadii,
    ) -> WidgetPaintOutput {
        let mut out = WidgetPaintOutput {
            owns_node_text: true,
            suppress_css_root_surface: true,
            geometry_on_page: self.open,
            ..WidgetPaintOutput::default()
        };
        let matching_text_paint = |target: [f32; 4]| {
            crate::widget_visual::matching_paint_color(style.paint_colors.color, target)
        };
        if let Some(chrome) = geometry.trigger_surface
            && (chrome.background.is_some() || chrome.border.is_some())
        {
            let corner_radius = if geometry.trigger_image.is_some() {
                [chrome.bounds.width.min(chrome.bounds.height) * 0.5; 4]
            } else if layout.border_radius.is_some() || layout.paint.border_radii.is_some() {
                let mut radii =
                    layout.resolved_border_radii(chrome.bounds.width, chrome.bounds.height);
                if let Some(nana_ui_core::ClipPath::Inset(inset)) = layout.paint.clip_path.as_ref()
                {
                    let round = inset.resolve_round(chrome.bounds.width, chrome.bounds.height);
                    if round > 0.0 {
                        radii = radii.map(|radius| radius.max(round));
                    }
                }
                radii
            } else {
                [chrome_radii.sm; 4]
            };
            out.push(
                1,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: chrome.bounds,
                    fill: chrome.background,
                    border: chrome.border.map(|border| (border, 1.0)),
                    corner_radius,
                    ..WidgetQuad::default()
                }),
            );
        }
        if let Some((icon, icon_bounds)) = geometry.trigger_icon {
            let (color, paint_color) = if let Some(ref label) = geometry.trigger {
                let fallback = label.color.or(style.color);
                let paint = style
                    .paint_colors
                    .color
                    .filter(|paint| label.color.is_none_or(|c| c == paint.to_srgb()));
                (fallback, paint)
            } else {
                let menu_fg = style_model
                    .color(
                        theme
                            .recipes()
                            .foreground(nana_ui_core::ComponentRecipeId::Menu, false),
                    )
                    .as_rgba_array();
                (Some(menu_fg), matching_text_paint(menu_fg))
            };
            out.push(
                2,
                WidgetPaintOp::Icon(WidgetIcon {
                    bounds: icon_bounds,
                    glyph: WidgetGlyph::Icon(icon),
                    color,
                    paint_color,
                    align_to_adjacent_text: false,
                }),
            );
        }
        if let Some(ref trigger) = geometry.trigger {
            out.push(
                if geometry.trigger_icon.is_some() {
                    3
                } else {
                    2
                },
                WidgetPaintOp::Text(WidgetText {
                    bounds: trigger.bounds,
                    content: trigger.content.clone(),
                    color: trigger.color,
                    paint_color: trigger.color.and_then(matching_text_paint),
                    font_size: trigger.font_size,
                    font_weight: trigger.font_weight,
                    horizontal_alignment: TextHorizontalAlignment::Start,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ellipsis: false,
                    ..WidgetText::default()
                }),
            );
        }
        if geometry.surface.height > 1.0 && geometry.surface.width > 1.0 {
            let surface_radius = theme
                .recipes()
                .menu()
                .surface_radius
                .resolve(style_model.metrics);
            let quad = WidgetQuad {
                rect: geometry.surface,
                fill: Some(geometry.background),
                border: Some((geometry.border, 1.0)),
                corner_radius: [surface_radius; 4],
                elevation: Some(geometry.elevation),
                use_parent_clips: geometry.trigger_surface.is_none(),
                ..WidgetQuad::default()
            };
            if geometry.trigger_surface.is_some() {
                out.push_tagged(
                    WidgetTaggedOp::new(TRIGGERED_OVERLAY_SURFACE_SLOT, WidgetPaintOp::Quad(quad))
                        .with_floating_overlay(),
                );
            } else {
                out.push(0, WidgetPaintOp::Quad(quad));
            }
        }
        if let Some(field) = geometry.search_field {
            out.push(
                3,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: field,
                    fill: Some(geometry.background),
                    border: Some((geometry.border, 1.0)),
                    corner_radius: [chrome_radii.field; 4],
                    ..WidgetQuad::default()
                }),
            );
        }
        if let Some(ref search) = geometry.search {
            out.push(
                4,
                WidgetPaintOp::Text(WidgetText {
                    bounds: search.bounds,
                    content: search.content.clone(),
                    color: search.color,
                    paint_color: search.color.and_then(matching_text_paint),
                    font_size: search.font_size,
                    font_weight: search.font_weight,
                    horizontal_alignment: TextHorizontalAlignment::Start,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ellipsis: true,
                    ..WidgetText::default()
                }),
            );
        }
        for (index, option) in geometry.options.iter().enumerate() {
            if let Some(background) = option.background {
                out.push(
                    10u64.saturating_add(index as u64),
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect: option.bounds,
                        fill: Some(background),
                        corner_radius: [chrome_radii.menu_row; 4],
                        ..WidgetQuad::default()
                    }),
                );
            }
            if let Some(ref image) = option.image {
                push_component_image(
                    &mut out,
                    widget_collection_slot(CONTEXT_MENU_IMAGE_BACKDROPS, index),
                    widget_collection_slot(CONTEXT_MENU_IMAGES, index),
                    image,
                    |op| op,
                );
            }
            if let Some((icon, icon_bounds, color)) = option.icon {
                out.push(
                    80u64.saturating_add(index as u64),
                    WidgetPaintOp::Icon(WidgetIcon {
                        bounds: icon_bounds,
                        glyph: WidgetGlyph::Icon(icon),
                        color: Some(color),
                        paint_color: matching_text_paint(color),
                        align_to_adjacent_text: false,
                    }),
                );
            }
            out.push(
                40u64.saturating_add(index as u64),
                WidgetPaintOp::Text(WidgetText {
                    bounds: option.label.bounds,
                    content: option.label.content.clone(),
                    color: option.label.color,
                    paint_color: option.label.color.and_then(matching_text_paint),
                    font_size: option.label.font_size,
                    font_weight: option.label.font_weight,
                    horizontal_alignment: TextHorizontalAlignment::Start,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ellipsis: true,
                    rich_text: option
                        .rich_content
                        .as_ref()
                        .and_then(|c| c.rich_text().cloned()),
                    ..WidgetText::default()
                }),
            );
            if let Some(ref hint) = option.hint {
                out.push(
                    widget_collection_slot(CONTEXT_MENU_HINTS, index),
                    WidgetPaintOp::Text(WidgetText {
                        bounds: hint.bounds,
                        content: hint.content.clone(),
                        color: hint.color,
                        paint_color: hint.color.and_then(matching_text_paint),
                        font_size: hint.font_size,
                        font_weight: hint.font_weight,
                        horizontal_alignment: TextHorizontalAlignment::End,
                        vertical_alignment: TextVerticalAlignment::Center,
                        ellipsis: true,
                        ..WidgetText::default()
                    }),
                );
            }
        }
        out
    }
}

/// A triggered menu's paint over its published trigger parts: the label in
/// its `TextContent` lane, the glyph in its `LeadingContent` square
/// (`UiWorld::solve_content_parts_at`). Neither is placed here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn menu_surface_geometry(
    bounds: LayoutBox,
    trigger: Option<&Arc<str>>,
    trigger_icon: Option<Icon>,
    trigger_image: Option<&Arc<str>>,
    trigger_content: bool,
    trigger_labelled: bool,
    style: &crate::ComputedStyle,
    style_model: StyleModelRef,
    theme: &nana_ui_core::CompiledTheme,
    surface: LayoutBox,
    parts: &crate::layout_engine::parts::ContentParts,
) -> MenuSurfaceGeometry {
    use crate::LayoutPartKind;
    use crate::layout_engine::parts::part_bounds;
    let palette = &style_model.palette;
    let menu = theme.recipes().menu();
    let has_trigger =
        trigger.is_some() || trigger_icon.is_some() || trigger_image.is_some() || trigger_content;
    let trigger_h = if has_trigger { bounds.height } else { 0.0 };
    // In icon and image modes the trigger text is only the accessible name, so
    // no text region is emitted and the chrome owns the visuals.
    let label = if (trigger_icon.is_none() || trigger_labelled) && trigger_image.is_none() {
        trigger.filter(|value| !value.is_empty())
    } else {
        None
    };
    let trigger_bounds = LayoutBox {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: trigger_h,
    };
    MenuSurfaceGeometry {
        trigger: label
            .zip(part_bounds(parts, LayoutPartKind::TextContent))
            .map(|(value, bounds)| ComponentTextRegion {
                bounds,
                content: Arc::clone(value).into(),
                color: Some(style.color.unwrap_or_else(|| palette.text.as_rgba_array())),
                font_size: style.font_size,
                font_weight: None,
            }),
        trigger_icon: trigger_icon.zip(part_bounds(parts, LayoutPartKind::LeadingContent)),
        trigger_image: trigger_image.cloned(),
        // Hover and press already resolved into the computed style, so the
        // trigger reads its chrome from there rather than the raw palette.
        trigger_surface: has_trigger.then_some(crate::ComponentTriggerSurface {
            bounds: trigger_bounds,
            background: style.background,
            border: style.border_color,
        }),
        surface,
        search: None,
        search_field: None,
        options: Vec::new(),
        elevation: ComponentElevation::from_shadow(theme.effects().shadow(menu.surface_elevation)),
        background: style_model.color(menu.surface_background).as_rgba_array(),
        border: style_model.color(menu.surface_border).as_rgba_array(),
    }
}

/// What an anchored surface does when neither side of its anchor holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorOverflow {
    /// Keep its size and its side, and move into the room: it overlaps the
    /// anchor rather than leave the room (a menu, a popover).
    Shift,
    /// Take the roomier side and shrink to it, and shrink across to the
    /// room's width or height (a drop-down list, which scrolls).
    Cap,
}

/// The one placement every anchored surface uses: popovers, triggered and
/// pointer menus (through [`nana_ui_core::AnchorSpec`] in layout), and the
/// select drop-down.
///
/// The surface of `width` × `height` hangs from `anchor` on `placement`'s
/// side, `gap` away, with `alignment` along that side. It takes that side
/// if it fits there. Otherwise it flips to the opposite side if it fits
/// there. Otherwise `overflow` decides. Across the side it is then kept
/// inside `room`. A zero-sized `anchor` is a point.
#[allow(clippy::too_many_arguments)]
pub fn resolve_anchored_surface(
    anchor: LayoutBox,
    width: f32,
    height: f32,
    room: LayoutBox,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    gap: f32,
    overflow: AnchorOverflow,
) -> LayoutBox {
    let vertical = matches!(placement, PopoverPlacement::Top | PopoverPlacement::Bottom);
    // Along the side's axis: the anchor's span, the room's, the surface's
    // extent, and whether the preferred side is after the anchor.
    let (anchor_start, anchor_end, room_start, room_end, extent) = if vertical {
        (
            anchor.y,
            anchor.y + anchor.height,
            room.y,
            room.y + room.height,
            height,
        )
    } else {
        (
            anchor.x,
            anchor.x + anchor.width,
            room.x,
            room.x + room.width,
            width,
        )
    };
    let prefers_after = matches!(
        placement,
        PopoverPlacement::Bottom | PopoverPlacement::Right
    );
    let space_after = (room_end - (anchor_end + gap)).max(0.0);
    let space_before = ((anchor_start - gap) - room_start).max(0.0);
    let (preferred, other) = if prefers_after {
        (space_after, space_before)
    } else {
        (space_before, space_after)
    };
    let (after, extent) = if extent <= preferred {
        (prefers_after, extent)
    } else if extent <= other {
        (!prefers_after, extent)
    } else {
        match overflow {
            AnchorOverflow::Shift => (prefers_after, extent),
            AnchorOverflow::Cap if space_after >= space_before => (true, space_after),
            AnchorOverflow::Cap => (false, space_before),
        }
    };
    let mut main = if after {
        anchor_end + gap
    } else {
        anchor_start - gap - extent
    };
    if overflow == AnchorOverflow::Shift {
        main = main.clamp(room_start, (room_end - extent).max(room_start));
    }
    // Across the side: aligned to the anchor, then kept in the room.
    let (cross_anchor, cross_anchor_extent, cross_room, cross_room_extent, mut cross_extent) =
        if vertical {
            (anchor.x, anchor.width, room.x, room.width, width)
        } else {
            (anchor.y, anchor.height, room.y, room.height, height)
        };
    if overflow == AnchorOverflow::Cap {
        cross_extent = cross_extent.min(cross_room_extent.max(0.0));
    }
    let cross = match alignment {
        PopoverAlignment::Start => cross_anchor,
        PopoverAlignment::Center => cross_anchor + cross_anchor_extent / 2.0 - cross_extent / 2.0,
        PopoverAlignment::End => cross_anchor + cross_anchor_extent - cross_extent,
    }
    .clamp(
        cross_room,
        (cross_room + cross_room_extent - cross_extent).max(cross_room),
    );
    if vertical {
        LayoutBox {
            x: cross,
            y: main,
            width: cross_extent,
            height: extent,
        }
    } else {
        LayoutBox {
            x: main,
            y: cross,
            width: extent,
            height: cross_extent,
        }
    }
}

/// Where an anchored surface of `surface_width` × `surface_height` opens
/// against `trigger` inside `viewport`: [`resolve_anchored_surface`] with
/// [`AnchorOverflow::Shift`]. The surface flips to the opposite side when
/// only that side has room for it, as a tooltip does, and is kept inside the
/// viewport. A surface with room on neither side overlaps its trigger rather
/// than leave the window.
pub fn resolve_popover_origin(
    trigger: LayoutBox,
    surface_width: f32,
    surface_height: f32,
    viewport: LayoutBox,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    gap: f32,
) -> (f32, f32) {
    let surface = resolve_anchored_surface(
        trigger,
        surface_width,
        surface_height,
        viewport,
        placement,
        alignment,
        gap,
        AnchorOverflow::Shift,
    );
    (surface.x, surface.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::LayoutViewport;
    use crate::framework::AppContext;
    use std::time::Duration;

    /// The popover's keyboard contract through the real key router: Enter on
    /// the trigger opens it, Escape closes it and gives focus back to the
    /// trigger, and with `close_on_escape(false)` Escape leaves it open.
    #[test]
    fn enter_opens_and_escape_closes_a_popover_from_the_keyboard() {
        use crate::view::{entity_ref, widget, with_refs};
        use crate::{Button, Stack};
        use nana_ui_input::{KeyInput, KeyState};
        fn press(context: &mut AppContext, key: &'static str) {
            let mut input = crate::HeadlessInput::bind(context, document());
            input
                .press(
                    context,
                    KeyInput::named(key, key, KeyState::Pressed, Default::default()),
                    None,
                    None,
                )
                .unwrap();
        }

        let mut context = AppContext::new();
        let root = context
            .create_component(document(), Stack::column(0.0))
            .unwrap();
        let (_, (popover, item)) = context
            .mount_view(root.stable_id(), || {
                let popover = entity_ref::<Popover>();
                let item = entity_ref::<Button>();
                with_refs(
                    widget(Popover::new().trigger("Options"))
                        .entity_ref(popover)
                        .children(widget(Button::new("Rename")).entity_ref(item)),
                    (popover, item),
                )
            })
            .unwrap();
        let layout = |context: &mut AppContext| {
            context
                .layout_document(document(), LayoutViewport::new(400.0, 300.0))
                .unwrap();
        };
        let open = |context: &AppContext| context.read(popover, |popover| popover.open).unwrap();
        layout(&mut context);
        assert!(context.focus_node(document(), popover.stable_id()).unwrap());
        press(&mut context, "Enter");
        layout(&mut context);
        assert!(open(&context));

        // From inside the panel, Escape closes it and focus returns.
        assert!(context.focus_node(document(), item.stable_id()).unwrap());
        press(&mut context, "Escape");
        assert!(!open(&context));
        assert_eq!(
            context.world().focused(document()),
            Some(popover.stable_id())
        );

        // A popover that keeps itself open on Escape does.
        context
            .update_component(popover, |popover, _| popover.close_on_escape = false)
            .unwrap();
        press(&mut context, "Enter");
        layout(&mut context);
        assert!(open(&context));
        press(&mut context, "Escape");
        assert!(open(&context));
    }

    #[test]
    fn compact_heading_tools_keep_their_targets_and_menu_anchor() {
        use crate::view::{entity_ref, widget, with_refs};
        use crate::{IconButton, Stack};
        use nana_ui_core::AlignSpec;

        let mut context = AppContext::new();
        let root = context
            .create_component(document(), Stack::row(2.0).align(AlignSpec::Center))
            .unwrap();
        let (_, (button, popover, menu)) = context
            .mount_view(root.stable_id(), || {
                let button = entity_ref::<IconButton>();
                let popover = entity_ref::<Popover>();
                let menu = entity_ref::<ActionMenu>();
                with_refs(
                    (
                        widget(IconButton::new(Icon::Add, "Add").compact(true)).entity_ref(button),
                        widget(
                            Popover::new()
                                .trigger_icon(Icon::Add, "Options")
                                .compact_trigger(true),
                        )
                        .entity_ref(popover),
                        widget(
                            ActionMenu::new()
                                .trigger_icon(Icon::Add, "More")
                                .compact_trigger(true)
                                .bare_trigger(true),
                        )
                        .entity_ref(menu)
                        .children(widget(crate::ActionMenuItem::new("Command"))),
                    ),
                    (button, popover, menu),
                )
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(280.0, 200.0))
            .unwrap();
        let ids = [button.stable_id(), popover.stable_id(), menu.stable_id()];
        let bounds: Vec<_> = ids
            .iter()
            .map(|id| context.world().layout_box(*id).unwrap())
            .collect();
        for bounds in &bounds {
            assert_eq!((bounds.width, bounds.height), (24.0, 24.0));
        }
        assert_eq!(
            context.world().layout_box(root.stable_id()).unwrap().height,
            24.0
        );
        assert!(context.activate_node(menu.stable_id()).unwrap());
        context
            .layout_document(document(), LayoutViewport::new(280.0, 200.0))
            .unwrap();
        assert!(context.read(menu, |menu| menu.popover.open).unwrap());
        assert_eq!(
            context.world().layout_box(menu.stable_id()).unwrap(),
            bounds[2]
        );
        context
            .update_component(button, |button, _| button.disabled = true)
            .unwrap();
        assert!(!context.activate_icon_button(button).unwrap());
        context
            .update_component(button, |button, _| {
                button.disabled = false;
                button.compact = false;
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(280.0, 200.0))
            .unwrap();
        assert_eq!(
            context
                .world()
                .layout_box(button.stable_id())
                .unwrap()
                .height,
            UI_METRICS.icon_button_size
        );
    }

    #[test]
    fn triggers_follow_the_parent_rows_cross_axis_alignment() {
        use crate::view::{entity_ref, widget, with_refs};
        use nana_ui_core::AlignSpec;

        for align in [AlignSpec::Start, AlignSpec::Center, AlignSpec::End] {
            for bare in [false, true] {
                let mut context = AppContext::new();
                let root = context
                    .create_component(
                        document(),
                        crate::Stack::row(8.0)
                            .width(LengthSpec::Px(400.0))
                            .height(LengthSpec::Px(48.0))
                            .padding(6.0)
                            .align(align),
                    )
                    .unwrap();
                let (_, (text, icon, content, menu)) = context
                    .mount_view(root.stable_id(), || {
                        let text = entity_ref::<Popover>();
                        let icon = entity_ref::<Popover>();
                        let content = entity_ref::<Popover>();
                        let menu = entity_ref::<ActionMenu>();
                        with_refs(
                            (
                                widget(Popover::new().trigger("Text").bare_trigger(bare))
                                    .entity_ref(text),
                                widget(
                                    Popover::new()
                                        .trigger_icon(Icon::Add, "Icon")
                                        .bare_trigger(bare),
                                )
                                .entity_ref(icon),
                                widget(Popover::new().trigger("Content").bare_trigger(bare))
                                    .entity_ref(content)
                                    .trigger(widget(
                                        crate::Stack::row(0.0)
                                            .width(LengthSpec::Px(14.0))
                                            .height(LengthSpec::Px(14.0)),
                                    )),
                                widget(ActionMenu::new().trigger("Menu").bare_trigger(bare))
                                    .entity_ref(menu),
                            ),
                            (text, icon, content, menu),
                        )
                    })
                    .unwrap();
                context
                    .layout_document(document(), LayoutViewport::new(400.0, 100.0))
                    .unwrap();
                let expected_y = match align {
                    AlignSpec::Start => 6.0,
                    AlignSpec::Center => 10.0,
                    AlignSpec::End => 14.0,
                    _ => unreachable!(),
                };
                for id in [
                    text.stable_id(),
                    icon.stable_id(),
                    content.stable_id(),
                    menu.stable_id(),
                ] {
                    let bounds = context.world().layout_box(id).unwrap();
                    assert!((bounds.height - TRIGGER_HEIGHT).abs() < 0.01, "{bounds:?}");
                    assert!(
                        (bounds.y - expected_y).abs() < 0.01,
                        "{align:?}, bare={bare}: {bounds:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn content_triggers_keep_their_intrinsic_width_in_stretch_columns() {
        use crate::view::widget;

        for bare in [false, true] {
            let mut context = AppContext::new();
            let root = context
                .create_component(
                    document(),
                    crate::Stack::column(0.0).width(LengthSpec::Fill),
                )
                .unwrap();
            context
                .mount_view(root.stable_id(), || {
                    widget(Popover::new().trigger("Content").bare_trigger(bare)).trigger(widget(
                        crate::Stack::row(0.0)
                            .width(LengthSpec::Px(40.0))
                            .height(LengthSpec::Px(14.0)),
                    ))
                })
                .unwrap();
            context
                .layout_document(document(), LayoutViewport::new(400.0, 100.0))
                .unwrap();
            let trigger = context.world().node(root.stable_id()).unwrap().children[0];
            let bounds = context.world().layout_box(trigger).unwrap();
            let expected_width = 40.0
                + UI_METRICS.control_padding_x * 2.0
                + if bare {
                    0.0
                } else {
                    nana_ui_core::HAIRLINE * 2.0
                };
            assert!(
                (bounds.width - expected_width).abs() < 0.01,
                "bare={bare}: {bounds:?}"
            );
        }
    }

    /// The hanging surface opens `gap` below the trigger's box, not below
    /// the first frame of the trigger's open animation.
    #[test]
    fn the_surface_hangs_a_gap_below_the_trigger_box() {
        let mut context = AppContext::new();
        let popover = context
            .create_component(document(), Popover::new().trigger("Details").open(true))
            .unwrap();
        let body = context
            .create_component(document(), crate::Text::new("Inspector content"))
            .unwrap();
        context.append_child(popover, body).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(420.0, 200.0))
            .unwrap();
        let trigger = context.world().layout_box(popover.stable_id()).unwrap();
        let surface = hanging_surface(context.world(), popover.stable_id()).unwrap();
        assert!(
            (surface.y - (trigger.y + trigger.height + POPOVER_GAP)).abs() < 0.01,
            "trigger {trigger:?}, surface {surface:?}"
        );
    }

    /// An open popover's hanging surface is sized from its items: an item
    /// whose box changes repaints and re-hit-tests the surface too.
    #[test]
    fn an_item_that_resizes_repaints_the_hanging_surface() {
        let mut context = AppContext::new();
        let popover = context
            .create_component(
                document(),
                Popover::new().trigger("Details").width(240.0).open(true),
            )
            .unwrap();
        let body = context
            .create_component(
                document(),
                crate::Stack::column(0.0).height(LengthSpec::Px(40.0)),
            )
            .unwrap();
        context.append_child(popover, body).unwrap();
        let viewport = LayoutViewport::new(320.0, 400.0);
        context.layout_document(document(), viewport).unwrap();
        let _ = context.take_system_work();

        context
            .update_component(body, |stack, _| {
                *stack = crate::Stack::column(0.0).height(LengthSpec::Px(120.0));
            })
            .unwrap();
        context.layout_document(document(), viewport).unwrap();
        let work = context.take_system_work();
        assert!(work.render_extraction.contains(&popover.stable_id()));
        assert!(work.input_hit_test.contains(&popover.stable_id()));
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// The card is the union of its items' boxes. Once the close motion has
    /// finished, that union is empty: the items have left the branch, and a
    /// leftover box must not keep the surface painted on the trigger.
    #[test]
    fn a_closed_menu_drops_its_hanging_surface() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                ActionMenu::new().trigger_icon(crate::Icon::Settings, "播放设置"),
            )
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("剧场"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        let viewport = LayoutViewport::new(480.0, 800.0);
        let layout = |context: &mut AppContext| {
            context.layout_document(document(), viewport).unwrap();
        };
        layout(&mut context);
        context.toggle_action_menu(menu).unwrap();
        layout(&mut context);
        let open_surface = context
            .world()
            .menu_surface_geometry(menu.stable_id())
            .expect("open menu geometry")
            .surface;
        assert!(
            open_surface.width > 1.0 && open_surface.height > 1.0,
            "{open_surface:?}"
        );
        context.toggle_action_menu(menu).unwrap();
        context.advance_animations(Duration::from_millis(400));
        layout(&mut context);
        let closed_surface = context
            .world()
            .menu_surface_geometry(menu.stable_id())
            .expect("closed menu geometry")
            .surface;
        assert!(
            closed_surface.width <= 1.0 && closed_surface.height <= 1.0,
            "card still painted: {closed_surface:?}"
        );
        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn choosing_an_item_closes_the_menu_that_offered_it() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("Actions").open(true))
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        assert!(context.activate_action_menu_item(item).unwrap());
        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn first_action_item_beats_later_siblings_under_bottom_placement() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                crate::ActionMenu::new().trigger("Actions").open(true),
            )
            .unwrap();
        let first = context
            .create_component(document(), crate::ActionMenuItem::new("First"))
            .unwrap();
        let last = context
            .create_component(document(), crate::ActionMenuItem::new("Last"))
            .unwrap();
        context.append_child(menu, first).unwrap();
        context.append_child(menu, last).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        context.rebuild_hit_test(document());
        let first_id = first.stable_id();
        let first_box = context.world().component_layout_box(first_id).unwrap();
        let last_box = context
            .world()
            .canonical_layout_box(last.stable_id())
            .unwrap();
        assert!(
            last_box.y > first_box.y,
            "items stack below the trigger: first={first_box:?} last={last_box:?}"
        );
        let hit = context.pointer_target(
            document(),
            first_box.x + first_box.width / 2.0,
            first_box.y + first_box.height / 2.0,
        );
        assert_eq!(
            hit,
            Some(first_id),
            "first={first_box:?} last={last_box:?} hit={hit:?}"
        );
    }

    /// The trigger is the only pressable affordance a closed menu has, so it
    /// must carry its own background rather than reading as bare text.
    #[test]
    fn a_closed_trigger_paints_pressable_button_chrome() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let id = StableNodeId::new(1).unwrap();
        queue.create(
            id,
            document(),
            NodeKind::Element {
                tag: "action-menu".into(),
            },
        );
        queue.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(id, trigger_button_style());
        // The label is the node's text, as the projection writes it; layout
        // publishes its lane as the trigger's text part (#205).
        queue.set_text(
            id,
            crate::TextContent {
                value: "Actions".into(),
            },
        );
        queue.set_widget_visual(
            id,
            Some(MenuSurfaceVisual {
                kind: MenuSurfaceKind::ActionMenu,
                open: false,
                trigger: Some(Arc::from("Actions")),
                trigger_icon: None,
                trigger_image: None,
                gap: 0.0,
                overlay: None,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_styles(&[id]).unwrap();
        let geometry = world
            .menu_surface_geometry(id)
            .expect("expected menu surface geometry");
        let chrome = geometry.trigger_surface.expect("trigger chrome");
        let idle = chrome.background.expect("trigger has a filled surface");
        assert_eq!(chrome.bounds.height, TRIGGER_HEIGHT);
        let label = geometry.trigger.expect("trigger label");
        assert!(label.bounds.x > 0.0, "label sits inside the button padding");

        world.set_pointer_hover(document(), 1, Some(id)).unwrap();
        world.advance_animations(nana_ui_core::motion::HOVER_COLOR);
        world.resolve_styles(&[id]).unwrap();
        let geometry = world
            .menu_surface_geometry(id)
            .expect("expected menu surface geometry");
        let hovered = geometry
            .trigger_surface
            .expect("trigger chrome")
            .background
            .expect("hovered trigger stays filled");
        assert_ne!(idle, hovered, "the trigger answers the pointer");
    }

    /// The trigger and the surface share one box, so a closed menu whose items
    /// still took part in layout would be stretched to their width.
    #[test]
    fn a_closed_menu_keeps_its_items_out_of_the_layout() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("Actions"))
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        let omits_box = |context: &AppContext| {
            context
                .world()
                .layout_style(item.stable_id())
                .unwrap()
                .omits_box()
        };
        assert!(omits_box(&context));
        context.toggle_action_menu(menu).unwrap();
        assert!(!omits_box(&context));
    }

    #[test]
    fn popover_closed_keeps_the_trigger_and_open_reserves_surface_padding() {
        let mut context = AppContext::new();
        let popover = context
            .create_component(document(), Popover::new().trigger("Details"))
            .unwrap();
        let id = popover.stable_id();
        assert!(matches!(
            context.world().widget_visual_as::<MenuSurfaceVisual>(id),
            Some(MenuSurfaceVisual {
                kind: MenuSurfaceKind::Popover,
                trigger: Some(label),
                ..
            }) if label.as_ref() == "Details"
        ));
        let closed = context.world().node_style(id).unwrap();
        assert!(!closed.layout.hidden);
        assert_eq!(
            closed.control_height,
            Some(nana_ui_core::ControlHeight::Exact(
                nana_ui_core::ControlSize::Small
            ))
        );
        assert_eq!(
            context.world().extract_nodes(&[id])[0]
                .source_style
                .layout
                .height,
            Some(LengthSpec::Px(TRIGGER_HEIGHT))
        );
        // A closed trigger is a pressable button, not bare text.
        assert_eq!(closed.background, Some(SemanticColorRole::Subtle));
        assert_eq!(closed.border, Some(SemanticColorRole::BorderSoft));
        assert!(closed.interaction.hovered.background.is_some());
        assert!(closed.interaction.pressed.background.is_some());
        assert!(
            matches!(closed.layout.width, None | Some(LengthSpec::Shrink)),
            "trigger sizes to its label"
        );
        context
            .update_component(popover, |popover, _| {
                popover.open = true;
            })
            .unwrap();
        let open = context.world().node_style(id).unwrap();
        assert_eq!(
            context.world().extract_nodes(&[id])[0]
                .source_style
                .layout
                .height,
            Some(LengthSpec::Px(TRIGGER_HEIGHT))
        );
        assert_eq!(open.background, Some(SemanticColorRole::Subtle));
        assert_eq!(open.layout.position, PositionSpec::Static);
    }

    /// A bare icon trigger is an empty glyph at rest. The wash appears only
    /// while the pointer is on it, and again while the surface is open.
    #[test]
    fn bare_icon_trigger_rests_without_a_fill_and_washes_on_hover() {
        let mut context = AppContext::new();
        let popover = context
            .create_component(
                document(),
                Popover::new()
                    .trigger_icon(Icon::Add, "更多")
                    .bare_trigger(true),
            )
            .unwrap();
        let id = popover.stable_id();
        let closed = context.world().node_style(id).unwrap();
        assert_eq!(closed.background, None);
        assert_eq!(closed.border, None);
        assert_eq!(closed.layout.border_width, Some(0.0));
        assert_eq!(
            closed.interaction.hovered.background,
            Some(SemanticColorRole::Hover)
        );
        assert_eq!(
            closed.interaction.pressed.background,
            Some(SemanticColorRole::Active)
        );
        let layout = &context.world().extract_nodes(&[id])[0].source_style.layout;
        assert_eq!(layout.min_width, Some(LengthSpec::Px(TRIGGER_HEIGHT)));
        assert_eq!(layout.min_height, Some(LengthSpec::Px(TRIGGER_HEIGHT)));

        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let geometry_id = StableNodeId::new(1).unwrap();
        queue.create(
            geometry_id,
            document(),
            NodeKind::Element {
                tag: "popover".into(),
            },
        );
        queue.write_layout(
            geometry_id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: TRIGGER_HEIGHT,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(
            geometry_id,
            bare_trigger_style(trigger_icon_button_style(), false),
        );
        queue.set_widget_visual(
            geometry_id,
            Some(MenuSurfaceVisual {
                kind: MenuSurfaceKind::Popover,
                open: false,
                trigger: None,
                trigger_icon: Some(Icon::Add),
                trigger_image: None,
                gap: 0.0,
                overlay: None,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_styles(&[geometry_id]).unwrap();
        let chrome = world
            .menu_surface_geometry(geometry_id)
            .expect("expected menu surface geometry")
            .trigger_surface
            .expect("trigger chrome");
        assert!(
            chrome.background.is_none(),
            "a bare trigger has no resting fill"
        );
        assert!(chrome.border.is_none(), "a bare trigger has no border");

        world
            .set_pointer_hover(document(), 1, Some(geometry_id))
            .unwrap();
        world.advance_animations(nana_ui_core::motion::HOVER_COLOR);
        world.resolve_styles(&[geometry_id]).unwrap();
        let trigger_surface = world
            .menu_surface_geometry(geometry_id)
            .expect("expected menu surface geometry")
            .trigger_surface;
        assert!(
            trigger_surface
                .expect("trigger chrome")
                .background
                .is_some(),
            "hover washes the bare trigger"
        );

        context
            .update_component(popover, |popover, _| {
                popover.open = true;
            })
            .unwrap();
        let open = context.world().node_style(id).unwrap();
        assert_eq!(open.background, Some(SemanticColorRole::Hover));
        assert_eq!(open.border, None);
    }

    #[test]
    fn trigger_icon_swaps_the_label_for_an_accessible_glyph_trigger() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                ActionMenu::new().trigger_icon(Icon::Add, "添加"),
            )
            .unwrap();
        let id = menu.stable_id();
        assert!(matches!(
            context.world().widget_visual_as::<MenuSurfaceVisual>(id),
            Some(MenuSurfaceVisual {
                trigger: Some(trigger),
                trigger_icon: Some(icon),
                ..
            }) if trigger.as_ref() == "添加" && *icon == Icon::Add
        ));
        // The label stays the accessible name only; the icon trigger measures
        // no text and its hit target is the square trigger box.
        assert!(context.world().text(id).is_none_or(|text| text.is_empty()));
        let layout = &context.world().extract_nodes(&[id])[0].source_style.layout;
        assert_eq!(layout.min_width, Some(LengthSpec::Px(TRIGGER_HEIGHT)));
        assert_eq!(layout.min_height, Some(LengthSpec::Px(TRIGGER_HEIGHT)));
    }

    /// The icon trigger's glyph must center geometrically in the chrome, not
    /// ride text line metrics — bare symbols ride high inside their em box.
    #[test]
    fn an_icon_trigger_centers_its_glyph_in_the_chrome() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let id = StableNodeId::new(1).unwrap();
        queue.create(
            id,
            document(),
            NodeKind::Element {
                tag: "action-menu".into(),
            },
        );
        queue.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: TRIGGER_HEIGHT,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(id, trigger_icon_button_style());
        queue.set_widget_visual(
            id,
            Some(MenuSurfaceVisual {
                kind: MenuSurfaceKind::ActionMenu,
                open: false,
                trigger: None,
                trigger_icon: Some(Icon::Add),
                trigger_image: None,
                gap: 0.0,
                overlay: None,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_styles(&[id]).unwrap();
        let MenuSurfaceGeometry {
            trigger,
            trigger_icon,
            trigger_surface,
            ..
        } = world
            .menu_surface_geometry(id)
            .expect("expected menu surface geometry");
        assert!(trigger.is_none(), "icon trigger carries no text region");
        let (icon, icon_bounds) = trigger_icon.expect("trigger glyph box");
        assert_eq!(icon, Icon::Add);
        let chrome = trigger_surface.expect("trigger chrome").bounds;
        assert_eq!(
            icon_bounds.x + icon_bounds.width / 2.0,
            chrome.x + chrome.width / 2.0
        );
        assert_eq!(
            icon_bounds.y + icon_bounds.height / 2.0,
            chrome.y + chrome.height / 2.0
        );
    }

    /// `trigger_icon_size` reaches the glyph box through the trigger's
    /// overlay, centred like the standard glyph; without it the glyph keeps
    /// the standard size.
    #[test]
    fn an_icon_triggers_glyph_takes_the_size_it_was_given() {
        let glyph_for = |size: Option<f32>| {
            let mut world = UiWorld::new();
            let mut queue = MutationQueue::new();
            let id = StableNodeId::new(1).unwrap();
            queue.create(
                id,
                document(),
                NodeKind::Element {
                    tag: "action-menu".into(),
                },
            );
            queue.write_layout(
                id,
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: TRIGGER_HEIGHT,
                    height: TRIGGER_HEIGHT,
                },
            );
            queue.set_style(id, trigger_icon_button_style());
            queue.set_widget_visual(
                id,
                Some(MenuSurfaceVisual {
                    kind: MenuSurfaceKind::ActionMenu,
                    open: false,
                    trigger: None,
                    trigger_icon: Some(Icon::Add),
                    trigger_image: None,
                    gap: 0.0,
                    overlay: Some(crate::TriggeredMenuOverlay {
                        placement: PopoverPlacement::Bottom,
                        alignment: PopoverAlignment::Start,
                        width: ACTION_MENU_WIDTH,
                        padding: 0.0,
                        gap: 0.0,
                        trigger_content: None,
                        trigger_icon_size: size,
                        trigger_label_gap: None,
                        overlay_mount: nana_ui_core::OverlayMount::NearestModal,
                    }),
                    query: None,
                    rows: Arc::from([]),
                    highlighted: None,
                }),
            );
            world.commit(queue).unwrap();
            world.resolve_styles(&[id]).unwrap();
            let trigger_icon = world
                .menu_surface_geometry(id)
                .expect("expected menu surface geometry")
                .trigger_icon;
            trigger_icon.expect("trigger glyph box").1
        };
        let standard = glyph_for(None);
        assert_eq!(standard.width, TRIGGER_ICON_SIZE.min(TRIGGER_HEIGHT));
        let sized = glyph_for(Some(16.0));
        assert_eq!((sized.width, sized.height), (16.0, 16.0));
        assert_eq!(sized.x + sized.width / 2.0, TRIGGER_HEIGHT / 2.0);
        assert_eq!(sized.y + sized.height / 2.0, TRIGGER_HEIGHT / 2.0);
        // the builder forwards it, and ignores a size that is not one
        let menu = ActionMenu::new()
            .trigger_icon_size(16.0)
            .trigger_icon_size(-1.0);
        assert_eq!(menu.popover.trigger_icon_size, Some(16.0));
    }

    #[test]
    fn action_menu_defaults_start_alignment_and_compact_padding() {
        let menu = ActionMenu::new();
        assert_eq!(menu.popover.alignment, PopoverAlignment::Start);
        assert_eq!(menu.popover.gap, ACTION_MENU_GAP);
        assert_eq!(menu.popover.width, ACTION_MENU_WIDTH);
        assert_eq!(menu.popover.padding, ACTION_MENU_PADDING);
    }

    #[test]
    fn popover_origin_flips_to_the_side_with_room_then_clamps() {
        let viewport = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 120.0,
        };
        let origin = |trigger, placement| {
            resolve_popover_origin(
                trigger,
                80.0,
                60.0,
                viewport,
                placement,
                PopoverAlignment::Center,
                6.0,
            )
        };
        // No room below the trigger, room above: it opens upward, and the
        // cross axis clamps into the viewport.
        let low = LayoutBox {
            x: 90.0,
            y: 80.0,
            width: 20.0,
            height: 20.0,
        };
        assert_eq!(origin(low, PopoverPlacement::Bottom), (40.0, 14.0));
        // No room to the left, room to the right: it opens rightward.
        let tight = LayoutBox {
            x: 4.0,
            y: 4.0,
            width: 20.0,
            height: 20.0,
        };
        assert_eq!(origin(tight, PopoverPlacement::Left), (30.0, 0.0));
        // Room on the requested side: no flip.
        assert_eq!(origin(tight, PopoverPlacement::Bottom), (0.0, 30.0));
        // Room on neither side: it keeps the requested side and is clamped
        // over the trigger rather than leaving the viewport.
        let middle = LayoutBox {
            x: 50.0,
            y: 50.0,
            width: 20.0,
            height: 20.0,
        };
        assert_eq!(origin(middle, PopoverPlacement::Bottom), (20.0, 60.0));
    }

    /// An open menu's items leave the host row: the trigger keeps the closed
    /// box, neighbours stay put, and the panel hangs below the trigger.
    #[test]
    fn an_open_menu_stops_participating_in_its_host_layout() {
        let mut context = AppContext::new();
        let row = context
            .create_component(document(), crate::Stack::row(8.0))
            .unwrap();
        let sibling = context
            .create_component(document(), crate::Button::new("侧边"))
            .unwrap();
        let slot = context
            .create_component(document(), crate::Stack::column(4.0))
            .unwrap();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("询问"))
            .unwrap();
        context.append_child(row, sibling).unwrap();
        context.append_child(row, slot).unwrap();
        context.append_child(slot, menu).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let closed_slot = context
            .world()
            .canonical_layout_box(slot.stable_id())
            .unwrap();
        let closed_sibling = context
            .world()
            .canonical_layout_box(sibling.stable_id())
            .unwrap();
        let closed_trigger = context
            .world()
            .canonical_layout_box(menu.stable_id())
            .unwrap();

        context
            .update_component(menu, |menu, _| {
                menu.popover.open = true;
            })
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("只读"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();

        let open_trigger = context
            .world()
            .canonical_layout_box(menu.stable_id())
            .unwrap();
        let open_slot = context
            .world()
            .canonical_layout_box(slot.stable_id())
            .unwrap();
        let open_item = context
            .world()
            .canonical_layout_box(item.stable_id())
            .unwrap();
        let style = context.world().node_style(menu.stable_id()).unwrap();
        assert_eq!(style.layout.position, PositionSpec::Static);
        assert_eq!(open_trigger, closed_trigger);
        assert_eq!(open_slot, closed_slot);
        assert_eq!(
            context
                .world()
                .canonical_layout_box(sibling.stable_id())
                .unwrap(),
            closed_sibling
        );
        assert!(
            open_item.y >= open_trigger.y + open_trigger.height + ACTION_MENU_GAP - 1.0,
            "panel hangs below the trigger: trigger={open_trigger:?} item={open_item:?}"
        );
        let item_style = context.world().layout_style(item.stable_id()).unwrap();
        assert_eq!(item_style.position, PositionSpec::Fixed);
    }

    /// `Top` placement anchors the panel above the trigger: the trigger stays
    /// in place and the item column sits one gap above it.
    #[test]
    fn top_placement_opens_the_surface_above_its_slot() {
        let mut context = AppContext::new();
        let column = context
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let spacer = context
            .create_component(
                document(),
                crate::Stack::column(0.0).height(LengthSpec::Px(240.0)),
            )
            .unwrap();
        let row = context
            .create_component(document(), crate::Stack::row(8.0))
            .unwrap();
        let slot = context
            .create_component(document(), crate::Stack::column(4.0))
            .unwrap();
        let menu = context
            .create_component(
                document(),
                ActionMenu::new()
                    .trigger("工作树")
                    .placement(PopoverPlacement::Top),
            )
            .unwrap();
        context.append_child(column, spacer).unwrap();
        context.append_child(column, row).unwrap();
        context.append_child(row, slot).unwrap();
        context.append_child(slot, menu).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let closed_trigger = context
            .world()
            .canonical_layout_box(menu.stable_id())
            .unwrap();

        context
            .update_component(menu, |menu, _| {
                menu.popover.open = true;
            })
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("当前仓库"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();

        let open_trigger = context
            .world()
            .canonical_layout_box(menu.stable_id())
            .unwrap();
        let open_item = context
            .world()
            .canonical_layout_box(item.stable_id())
            .unwrap();
        assert_eq!(
            open_trigger, closed_trigger,
            "trigger stays in the host row"
        );
        assert!(
            open_item.y + open_item.height <= open_trigger.y - ACTION_MENU_GAP + 1.0,
            "panel opens above the trigger: trigger={open_trigger:?} item={open_item:?}"
        );
    }

    /// The `Menu` button kind paints the trigger chrome so an app-built menu
    /// button and an in-place action-menu trigger read as one control.
    #[test]
    fn menu_button_kind_paints_menu_trigger_chrome() {
        let mut context = AppContext::new();
        let button = context
            .create_component(
                document(),
                crate::Button::new("询问").kind(nana_ui_core::ButtonKind::Menu),
            )
            .unwrap();
        let style = context.world().node_style(button.stable_id()).unwrap();
        assert_eq!(
            style.background,
            Some(nana_ui_core::SemanticColorRole::Subtle)
        );
        assert_eq!(
            style.border,
            Some(nana_ui_core::SemanticColorRole::BorderSoft)
        );
        assert_eq!(
            style.interaction.hovered.background,
            Some(nana_ui_core::SemanticColorRole::Hover)
        );
    }

    #[test]
    fn scrolled_popover_overlay_hit_matches_accessibility_bounds() {
        let mut context = AppContext::new();
        let page = context
            .create_component(
                document(),
                crate::ScrollView::new(crate::ScrollAxes::Vertical).style(
                    crate::Stack::column(0.0)
                        .width(LengthSpec::Fill)
                        .height(LengthSpec::Px(240.0))
                        .node_style(),
                ),
            )
            .unwrap();
        let column = context
            .create_component(
                document(),
                crate::Stack::column(0.0).width(LengthSpec::Fill),
            )
            .unwrap();
        let spacer = context
            .create_component(
                document(),
                crate::Stack::column(0.0).height(LengthSpec::Px(400.0)),
            )
            .unwrap();
        let popover = context
            .create_component(
                document(),
                Popover::new().trigger("选择草稿").width(240.0).open(true),
            )
            .unwrap();
        context.append_child(page, column).unwrap();
        context.append_child(column, spacer).unwrap();
        context.append_child(column, popover).unwrap();
        let overlay = context
            .create_component(
                document(),
                crate::ScrollView::new(crate::ScrollAxes::Vertical).style(
                    crate::Stack::column(0.0)
                        .width(LengthSpec::Fill)
                        .height(LengthSpec::Px(200.0))
                        .node_style(),
                ),
            )
            .unwrap();
        let list = context
            .create_component(
                document(),
                crate::Stack::column(6.0).width(LengthSpec::Fill),
            )
            .unwrap();
        context.append_child(popover, overlay).unwrap();
        context.append_child(overlay, list).unwrap();
        let mut last = None;
        for index in 0..24 {
            let item = context
                .create_component(document(), crate::Button::new(format!("草稿 {index}")))
                .unwrap();
            context.append_child(list, item).unwrap();
            last = Some(item);
        }
        let last = last.unwrap();
        let viewport = LayoutViewport::new(320.0, 400.0);
        context.layout_document(document(), viewport).unwrap();
        context
            .scroll_to(page, crate::ScrollOffset { x: 0.0, y: 160.0 })
            .unwrap();
        context
            .scroll_to(
                overlay,
                crate::ScrollOffset {
                    x: 0.0,
                    y: 1_000_000.0,
                },
            )
            .unwrap();
        context.layout_document(document(), viewport).unwrap();
        context.rebuild_hit_test(document());
        assert_eq!(
            context.world().scroll_offset(page.stable_id()).map(|o| o.y),
            Some(160.0)
        );
        let last_id = last.stable_id();
        let bounds = context
            .world()
            .project_accessibility_in_viewport(document())
            .into_iter()
            .find(|entry| entry.id == last_id)
            .expect("last item is accessible")
            .bounds;
        let hit = context.pointer_target(
            document(),
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        assert_eq!(
            hit,
            Some(last_id),
            "overlay a11y center must ignore ancestor page scroll: {bounds:?} -> {hit:?}"
        );
    }

    /// The trigger's background, at rest and with the pointer over it, for a
    /// node styled `style`.
    fn trigger_wash(style: NodeStyle) -> ([f32; 4], [f32; 4]) {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let id = StableNodeId::new(1).unwrap();
        queue.create(
            id,
            document(),
            NodeKind::Element {
                tag: "popover".into(),
            },
        );
        queue.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(id, style);
        queue.set_widget_visual(
            id,
            Some(MenuSurfaceVisual {
                kind: MenuSurfaceKind::Popover,
                open: false,
                trigger: Some(Arc::from("Actions")),
                trigger_icon: None,
                trigger_image: None,
                gap: 0.0,
                overlay: None,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        fn background(world: &mut UiWorld, id: StableNodeId) -> [f32; 4] {
            world.resolve_styles(&[id]).unwrap();
            let chrome = world
                .menu_surface_geometry(id)
                .expect("expected trigger chrome")
                .trigger_surface
                .expect("expected trigger chrome");
            chrome.background.expect("a filled trigger")
        }
        let idle = background(&mut world, id);
        world.set_pointer_hover(document(), 1, Some(id)).unwrap();
        world.advance_animations(nana_ui_core::motion::HOVER_COLOR);
        (idle, background(&mut world, id))
    }

    #[test]
    fn a_trigger_style_is_written_over_the_recipe_not_instead_of_it() {
        let mut over = NodeStyle::default();
        over.layout_mut().width = Some(LengthSpec::Px(120.0));
        let mut context = AppContext::new();
        let popover = context
            .create_component(
                document(),
                Popover::new()
                    .trigger("Actions")
                    .trigger_style(over.clone()),
            )
            .unwrap();
        let id = popover.stable_id();
        let recipe = trigger_button_style();
        for open in [false, true, false] {
            context
                .update_component(popover, |popover, _| popover.open = open)
                .unwrap();
            let style = context.world().node_style(id).unwrap();
            assert_eq!(style.layout.width, Some(LengthSpec::Px(120.0)));
            // All it left unset is the recipe's: chrome, corners, height,
            // inset and the hover and press washes.
            assert_eq!(style.layout.border_width, recipe.layout.border_width);
            assert_eq!(
                (style.background, style.border, style.radius),
                (recipe.background, recipe.border, recipe.radius)
            );
            assert_eq!(
                (style.control_height, style.control_padding_x),
                (recipe.control_height, recipe.control_padding_x)
            );
            assert_eq!(style.interaction, recipe.interaction);
            // The surface keeps its own width and padding.
            assert_eq!(
                context
                    .read(popover, |popover| (popover.width, popover.padding))
                    .unwrap(),
                (POPOVER_WIDTH, POPOVER_PADDING)
            );
        }
        // And the trigger still answers the pointer.
        let (idle, hovered) = trigger_wash(context.world().node_style(id).unwrap().clone());
        assert_ne!(idle, hovered, "hover still washes the trigger");
        // A hover wash of its own replaces the recipe's, and only that one.
        over.interaction.hovered.background = Some(SemanticColorRole::Selected);
        let style = trigger_button_style().overlaid(&over);
        assert_eq!(
            style.interaction.hovered.background,
            Some(SemanticColorRole::Selected)
        );
        assert_eq!(
            style.interaction.pressed.background,
            Some(SemanticColorRole::Active)
        );
    }

    #[test]
    fn a_number_the_trigger_style_writes_wins_over_the_recipe_step() {
        let mut over = NodeStyle::default();
        {
            let layout = over.layout_mut();
            layout.border_radius = Some(18.0);
            layout.height = Some(LengthSpec::Px(36.0));
            layout.padding_left = Some(LengthSpec::Px(4.0));
        }
        let style = trigger_button_style().overlaid(&over);
        // A step would resolve over the number; the recipe's give way.
        assert_eq!(style.radius, None);
        assert_eq!(style.layout.border_radius, Some(18.0));
        assert_eq!(style.control_height, None);
        assert_eq!(style.layout.height, Some(LengthSpec::Px(36.0)));
        assert_eq!(style.control_padding_x, None);
        // Per-corner radii take the step's place too; a step of its own wins.
        let mut corners = NodeStyle::default();
        corners.layout_mut().paint.border_radii = Some([LengthSpec::Px(2.0); 4]);
        assert_eq!(trigger_button_style().overlaid(&corners).radius, None);
        let mut tier = NodeStyle::default();
        tier.radius = Some(nana_ui_core::RadiusTier::Lg);
        assert_eq!(
            trigger_button_style().overlaid(&tier).radius,
            Some(nana_ui_core::RadiusTier::Lg)
        );
    }

    #[test]
    fn a_trigger_style_keeps_a_compact_content_trigger_fitted_to_its_content() {
        let mut context = AppContext::new();
        let well = context
            .create_component(document(), crate::Stack::row(0.0))
            .unwrap();
        let mut over = NodeStyle::default();
        over.background = Some(SemanticColorRole::Selected);
        let popover = context
            .create_component(
                document(),
                Popover::new()
                    .trigger("Colour")
                    .trigger_content(well.stable_id())
                    .compact_trigger(true)
                    .trigger_style(over),
            )
            .unwrap();
        context.append_child(popover, well).unwrap();
        let style = context.world().node_style(popover.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Selected));
        assert_eq!(style.control_height, None, "still fitted");
        assert_eq!(style.control_padding_x, None, "still fitted");
        assert_eq!(
            style.interaction.hovered.background,
            Some(SemanticColorRole::Hover)
        );
    }

    #[test]
    fn a_layout_overlay_writes_only_the_fields_it_sets() {
        let mut base = NodeStyle::default();
        {
            let layout = base.layout_mut();
            layout.width = Some(LengthSpec::Px(80.0));
            layout.min_width = Some(LengthSpec::Percent(50.0));
            layout.align_items = nana_ui_core::AlignSpec::Center;
            layout.paint.border_radii = Some([LengthSpec::Px(3.0); 4]);
        }
        let mut over = NodeStyle::default();
        {
            let layout = over.layout_mut();
            layout.min_width = Some(LengthSpec::Px(40.0));
            layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
            layout.paint.border_radii = Some([LengthSpec::Px(6.0); 4]);
        }
        let style = base.clone().overlaid(&over);
        let layout = style.layout.as_ref();
        assert_eq!(layout.width, Some(LengthSpec::Px(80.0)));
        assert_eq!(
            layout.min_width,
            Some(LengthSpec::Px(40.0)),
            "another variant, whole"
        );
        assert_eq!(layout.align_items, nana_ui_core::AlignSpec::Center);
        assert_eq!(layout.overflow_x, nana_ui_core::OverflowSpec::Hidden);
        assert_eq!(layout.paint.border_radii, Some([LengthSpec::Px(6.0); 4]));
        // An empty overlay is the base.
        assert_eq!(base.clone().overlaid(&NodeStyle::default()), base);
    }
}
