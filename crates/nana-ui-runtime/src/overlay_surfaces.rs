use std::sync::Arc;

use nana_ui_core::{
    ControlSize, DialogClosePolicy, DialogSize, DrawerSide, LayoutStyle, LengthSpec, PositionSpec,
    UI_METRICS,
};

use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld, view_components::project_common,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModalSurfaceKind {
    Dialog(DialogSize),
    Confirm(DialogSize),
    Drawer(DrawerSide),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalInitialFocus {
    Surface,
    #[default]
    FirstAction,
    Target(StableNodeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModalBehavior {
    pub close_policy: DialogClosePolicy,
    pub initial_focus: ModalInitialFocus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModalSlots {
    /// An icon before the title, in the header row.
    pub title_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub footer: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub actions: Vec<StableNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmSlots {
    /// An icon before the title, in the header row.
    pub title_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub cancel: StableNodeId,
    pub secondary: Option<StableNodeId>,
    pub confirm: StableNodeId,
}

impl ConfirmSlots {
    pub(crate) fn modal_slots(&self) -> ModalSlots {
        ModalSlots {
            title_icon: self.title_icon,
            body: self.body,
            close_action: self.close_action,
            actions: std::iter::once(self.cancel)
                .chain(self.secondary)
                .chain([self.confirm])
                .collect(),
            ..ModalSlots::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmIntent {
    Cancel,
    Secondary,
    Confirm { danger: bool },
}

impl ModalSlots {
    pub(crate) fn ordered(&self) -> Vec<StableNodeId> {
        self.title_icon
            .into_iter()
            .chain(self.body)
            .chain(self.footer)
            .chain(self.close_action)
            .chain(self.actions.iter().copied())
            .collect()
    }
}

pub trait ModalSurface: ComponentView {
    fn slots(&self) -> &ModalSlots;
    fn slots_mut(&mut self) -> &mut ModalSlots;
    /// Whether the surface wants to be its host's open overlay. See
    /// [`crate::Dialog::open`].
    fn is_open(&self) -> bool;
    fn set_open(&mut self, open: bool);
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmDialog {
    pub title: Arc<str>,
    pub message: Arc<str>,
    /// Label of the confirming action built by
    /// [`AppContext::assemble_confirm_dialog`]; `None` says the framework's
    /// (`dialog.confirm`).
    pub confirm_label: Option<Arc<str>>,
    /// Label of the dismissing action built by
    /// [`AppContext::assemble_confirm_dialog`]; `None` says the framework's
    /// (`dialog.cancel`).
    pub cancel_label: Option<Arc<str>>,
    pub size: DialogSize,
    pub danger: bool,
    pub busy: bool,
    /// Open while it is in a tree under an [`crate::OverlayHost`]; see
    /// [`crate::Dialog::open`].
    pub open: bool,
    behavior: ModalBehavior,
    slots: ModalSlots,
    confirm_slots: Option<ConfirmSlots>,
    /// Slots given before the first assembly (a view's `.body`, `.cancel`…);
    /// [`AppContext::assemble_confirm_dialog`] makes the buttons missing here.
    requested: RequestedConfirmSlots,
    pub style: NodeStyle,
}

/// What a dialog was given before its first assembly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RequestedConfirmSlots {
    pub title_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub cancel: Option<StableNodeId>,
    pub secondary: Option<StableNodeId>,
    pub confirm: Option<StableNodeId>,
}

impl ConfirmDialog {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn size(mut self, size: DialogSize) -> Self {
        self.size = size;
        self
    }

    /// Label of the confirming action. Applications localize it here.
    pub fn confirm_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.confirm_label = Some(label.into());
        self
    }

    /// Label of the dismissing action. Applications localize it here.
    pub fn cancel_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.cancel_label = Some(label.into());
        self
    }

    /// A destructive confirmation: the confirming action the dialog makes
    /// takes the danger kind, and the title the theme's danger colour, the
    /// same tone a [`crate::Dialog::danger`] dialog speaks in.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Open while it is in a tree under an [`crate::OverlayHost`]; see
    /// [`crate::Dialog::open`].
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn new(title: impl Into<Arc<str>>, message: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_label: None,
            cancel_label: None,
            size: DialogSize::Default,
            danger: false,
            busy: false,
            open: false,
            behavior: ModalBehavior::default(),
            slots: ModalSlots::default(),
            confirm_slots: None,
            requested: RequestedConfirmSlots::default(),
            style: modal_root_style(),
        }
    }

    pub fn close_policy(mut self, close_policy: DialogClosePolicy) -> Self {
        self.behavior.close_policy = close_policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: ModalInitialFocus) -> Self {
        self.behavior.initial_focus = initial_focus;
        self
    }

    pub(crate) fn set_initial_focus(&mut self, initial_focus: ModalInitialFocus) {
        self.behavior.initial_focus = initial_focus;
    }

    pub fn behavior(&self) -> ModalBehavior {
        self.behavior
    }

    /// An icon before the title, placed on assembly.
    pub fn title_icon(mut self, icon: StableNodeId) -> Self {
        self.requested.title_icon = Some(icon);
        self
    }

    /// Content between the message and the actions, placed on assembly.
    pub fn body(mut self, body: StableNodeId) -> Self {
        self.requested.body = Some(body);
        self
    }

    /// The dismissing affordance beside the title, placed on assembly.
    pub fn close_action(mut self, close: StableNodeId) -> Self {
        self.requested.close_action = Some(close);
        self
    }

    /// The dismissing action; assembly makes one from
    /// [`Self::cancel_label`] when none is given.
    pub fn cancel(mut self, cancel: StableNodeId) -> Self {
        self.requested.cancel = Some(cancel);
        self
    }

    /// A third action between cancel and confirm, placed on assembly.
    pub fn secondary(mut self, secondary: StableNodeId) -> Self {
        self.requested.secondary = Some(secondary);
        self
    }

    /// The confirming action; assembly makes one from
    /// [`Self::confirm_label`] when none is given.
    pub fn confirm(mut self, confirm: StableNodeId) -> Self {
        self.requested.confirm = Some(confirm);
        self
    }

    pub(crate) fn requested_slots(&self) -> &RequestedConfirmSlots {
        &self.requested
    }

    pub fn confirm_slots(&self) -> Option<&ConfirmSlots> {
        self.confirm_slots.as_ref()
    }

    pub(crate) fn set_confirm_slots_state(&mut self, slots: ConfirmSlots) {
        self.confirm_slots = Some(slots);
    }
}

impl ModalSurface for ConfirmDialog {
    fn slots(&self) -> &ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut ModalSlots {
        &mut self.slots
    }
    fn is_open(&self) -> bool {
        self.open
    }
    fn set_open(&mut self, open: bool) {
        self.open = open;
    }
}

impl ComponentView for ConfirmDialog {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_confirm_dialog),
        lifecycle: Some(crate::AppContext::sync_modal_open::<Self>),
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
            tag: "confirm-dialog".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::AlertDialog,
            &self.title,
            None,
            Some(self.message.as_ref()),
            ModalSurfaceKind::Confirm(self.size),
            self.busy,
            self.danger,
            &self.slots,
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drawer {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub side: DrawerSide,
    /// Open while it is in a tree under an [`crate::OverlayHost`]; see
    /// [`crate::Dialog::open`].
    pub open: bool,
    behavior: ModalBehavior,
    slots: ModalSlots,
    pub style: NodeStyle,
}

impl Drawer {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            description: None,
            side: DrawerSide::Right,
            open: false,
            behavior: ModalBehavior::default(),
            slots: ModalSlots::default(),
            style: modal_root_style(),
        }
    }

    pub fn side(mut self, side: DrawerSide) -> Self {
        self.side = side;
        self
    }

    /// Open while it is in a tree under an [`crate::OverlayHost`]; see
    /// [`crate::Dialog::open`].
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn close_policy(mut self, close_policy: DialogClosePolicy) -> Self {
        self.behavior.close_policy = close_policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: ModalInitialFocus) -> Self {
        self.behavior.initial_focus = initial_focus;
        self
    }

    pub(crate) fn set_initial_focus(&mut self, initial_focus: ModalInitialFocus) {
        self.behavior.initial_focus = initial_focus;
    }

    pub fn behavior(&self) -> ModalBehavior {
        self.behavior
    }
}

impl ModalSurface for Drawer {
    fn slots(&self) -> &ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut ModalSlots {
        &mut self.slots
    }
    fn is_open(&self) -> bool {
        self.open
    }
    fn set_open(&mut self, open: bool) {
        self.open = open;
    }
}

impl ComponentView for Drawer {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_modal_slots::<Self>),
        lifecycle: Some(crate::AppContext::sync_modal_open::<Self>),
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
            tag: "drawer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Dialog,
            &self.title,
            self.description.as_deref(),
            None,
            ModalSurfaceKind::Drawer(self.side),
            false,
            false,
            &self.slots,
        );
    }
}

pub(crate) fn project_modal(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    role: AccessibilityRole,
    title: &Arc<str>,
    description: Option<&str>,
    body_text: Option<&str>,
    kind: ModalSurfaceKind,
    busy: bool,
    danger: bool,
    slots: &ModalSlots,
) {
    let visual = StandardVisual::ModalFrame {
        title: Arc::clone(title),
        description: description.map(Arc::from),
        body_text: body_text.map(Arc::from),
        kind,
        busy,
        danger,
        slots: slots.clone(),
    };
    if world.standard_visual(id) != Some(visual.clone()) {
        mutations.set_standard_visual(id, Some(visual));
    }
    project_common(
        id,
        world,
        mutations,
        style,
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
        AccessibilityState {
            role,
            label: Some(Arc::clone(title)),
            description: description.or(body_text).map(Arc::from),
            modal: true,
            busy,
            ..Default::default()
        },
    );
}

pub(crate) fn modal_root_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            position: PositionSpec::Fixed,
            width: Some(LengthSpec::Percent(100.0)),
            height: Some(LengthSpec::Percent(100.0)),
            z_index: Some(1_000),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

pub(crate) const DRAWER_WIDTH: f32 = 360.0;
pub(crate) const MODAL_PAD_X: f32 = nana_ui_core::space::XXXL;
pub(crate) const DRAWER_HEADER_PAD_Y: f32 = nana_ui_core::space::XXL;
pub(crate) const MODAL_BODY_PAD_TOP: f32 = nana_ui_core::space::MD;
pub(crate) const DRAWER_FOOTER_PAD_Y: f32 = nana_ui_core::space::XL;
pub(crate) const MODAL_TITLE_DESC_GAP: f32 = nana_ui_core::space::XS;
pub(crate) const MODAL_CLOSE_SIZE: f32 = ControlSize::Small.height_in(UI_METRICS);
pub(crate) const DRAWER_CLOSE_GAP: f32 = nana_ui_core::space::LG;
pub(crate) const MODAL_ACTION_GAP: f32 = nana_ui_core::space::MD;
pub(crate) const MODAL_ACTION_HEIGHT: f32 = ControlSize::Medium.height_in(UI_METRICS);
pub(crate) const MODAL_BODY_TEXT_SIZE: f32 = nana_ui_core::type_scale::BODY;

/// A drawer's title icon square: the built-in dialog's. A dialog's comes
/// from its theme recipe; a drawer keeps its own fixed chrome.
pub(crate) const DRAWER_ICON_SIZE: f32 = nana_ui_core::DialogRecipe::DEFAULT.icon_size;

/// A modal surface's chrome: the header row, the body's insets and the
/// footer band.
///
/// The header is one row laid out the way a flex row with
/// `align-items: center` lays it out: `[icon] title [close]`, each centred
/// in the row. A drawer's row is as tall as the tallest of the three, as it
/// always was. A dialog card's row is as tall as its title block, its icon
/// and its recipe's least height, but not its close button: a busy
/// confirmation hides that button, and the body must not jump when it
/// does.
///
/// A dialog card takes everything else from the theme's [`DialogRecipe`]
/// too: the icon and close squares and the gap between them, each section's
/// insets, the gap between actions and the hairlines under the header and
/// over the footer, which add to the band they close. A drawer keeps its
/// own fixed chrome.
///
/// [`DialogRecipe`]: nana_ui_core::DialogRecipe
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModalChrome {
    pub header_height: f32,
    pub footer_height: f32,
    pub body_pad_top: f32,
    pub body_pad_bottom: f32,
    /// Left and right inset of the header row, the body and the footer.
    pub header_x: f32,
    pub body_x: f32,
    pub footer_x: f32,
    /// Space between the footer's actions.
    pub action_gap: f32,
    row_top: f32,
    row_height: f32,
    text_height: f32,
    icon: Option<f32>,
    close: Option<f32>,
    gap: f32,
    /// From the footer band's top to its content: divider and inset.
    footer_top: f32,
    /// Where the actions start below the band's top: a drawer centres
    /// them in a band it gives the footer slot whole.
    action_top: f32,
    header_divider: f32,
    footer_divider: f32,
    drawer: bool,
}

impl ModalChrome {
    /// `hairline` is the installed theme's stroke, the thickness of a
    /// divider the recipe asks for.
    pub fn measure(
        kind: ModalSurfaceKind,
        title: crate::TextMetrics,
        description: Option<crate::TextMetrics>,
        slots: &ModalSlots,
        recipe: &nana_ui_core::DialogRecipe,
        hairline: f32,
    ) -> Self {
        let has_footer = slots.footer.is_some() || !slots.actions.is_empty();
        let drawer = matches!(kind, ModalSurfaceKind::Drawer(_));
        let text_height =
            title.height + description.map_or(0.0, |metrics| MODAL_TITLE_DESC_GAP + metrics.height);
        let (icon_size, close_size, gap) = if drawer {
            (DRAWER_ICON_SIZE, MODAL_CLOSE_SIZE, DRAWER_CLOSE_GAP)
        } else {
            (recipe.icon_size, recipe.close_size, recipe.header_gap)
        };
        let icon = slots.title_icon.map(|_| icon_size);
        let close = slots.close_action.map(|_| close_size);
        let row_height = text_height.max(icon.unwrap_or(0.0)).max(if drawer {
            close.unwrap_or(0.0)
        } else {
            recipe.header_min_height
        });
        if drawer {
            let footer_height = if has_footer {
                DRAWER_FOOTER_PAD_Y * 2.0 + MODAL_ACTION_HEIGHT
            } else {
                0.0
            };
            return Self {
                header_height: DRAWER_HEADER_PAD_Y * 2.0 + row_height,
                footer_height,
                body_pad_top: MODAL_BODY_PAD_TOP,
                body_pad_bottom: MODAL_BODY_PAD_TOP,
                header_x: MODAL_PAD_X,
                body_x: MODAL_PAD_X,
                footer_x: MODAL_PAD_X,
                action_gap: MODAL_ACTION_GAP,
                row_top: DRAWER_HEADER_PAD_Y,
                row_height,
                text_height,
                icon,
                close,
                gap,
                footer_top: 0.0,
                action_top: DRAWER_FOOTER_PAD_Y,
                header_divider: 0.0,
                footer_divider: 0.0,
                drawer,
            };
        }
        let header_divider = recipe.header_divider.map_or(0.0, |_| hairline);
        let footer_divider = if has_footer {
            recipe.footer_divider.map_or(0.0, |_| hairline)
        } else {
            0.0
        };
        let footer_top = footer_divider + recipe.footer.top;
        Self {
            header_height: recipe.header.top + row_height + recipe.header.bottom + header_divider,
            footer_height: if has_footer {
                footer_top + MODAL_ACTION_HEIGHT + recipe.footer.bottom
            } else {
                0.0
            },
            body_pad_top: recipe.body.top,
            body_pad_bottom: recipe.body_bottom(has_footer),
            header_x: recipe.header.inline,
            body_x: recipe.body.inline,
            footer_x: recipe.footer.inline,
            action_gap: recipe.action_gap,
            row_top: recipe.header.top,
            row_height,
            text_height,
            icon,
            close,
            gap,
            footer_top,
            action_top: footer_top,
            header_divider,
            footer_divider,
            drawer,
        }
    }

    /// How wide the title and its description may run: the header row less
    /// the icon and the close button with their gaps.
    pub fn text_width(self, surface_width: f32) -> f32 {
        let icon = self.icon.map_or(0.0, |size| size + self.gap);
        let close = self.close.map_or(0.0, |size| self.gap + size);
        (surface_width - self.header_x * 2.0 - icon - close).max(0.0)
    }

    /// Where the title block starts: after the icon, centred in the row.
    pub fn title_origin(self, surface: crate::LayoutBox) -> (f32, f32) {
        (
            surface.x + self.header_x + self.icon.map_or(0.0, |size| size + self.gap),
            surface.y + self.row_top + (self.row_height - self.text_height) / 2.0,
        )
    }

    pub fn chrome_height(self, body_content: f32) -> f32 {
        self.header_height
            + self.body_pad_top
            + body_content
            + self.body_pad_bottom
            + self.footer_height
    }

    pub fn body_box(self, surface: crate::LayoutBox) -> crate::LayoutBox {
        crate::LayoutBox {
            x: surface.x + self.body_x,
            y: surface.y + self.header_height + self.body_pad_top,
            width: (surface.width - self.body_x * 2.0).max(0.0),
            height: (surface.height
                - self.header_height
                - self.body_pad_top
                - self.body_pad_bottom
                - self.footer_height)
                .max(0.0),
        }
    }

    /// The square the title icon slot is placed in, at the row's start.
    pub fn icon_box(self, surface: crate::LayoutBox) -> Option<crate::LayoutBox> {
        let size = self.icon?;
        Some(crate::LayoutBox {
            x: surface.x + self.header_x,
            y: surface.y + self.row_top + (self.row_height - size) / 2.0,
            width: size,
            height: size,
        })
    }

    /// The square the close button is placed in, at the row's end.
    pub fn close_box(self, surface: crate::LayoutBox) -> Option<crate::LayoutBox> {
        let size = self.close?;
        Some(crate::LayoutBox {
            x: surface.x + surface.width - self.header_x - size,
            y: surface.y + self.row_top + (self.row_height - size) / 2.0,
            width: size,
            height: size,
        })
    }

    /// The footer's content box, where its row of actions and its slot
    /// sit: inside the divider and the insets. A drawer gives its footer
    /// slot the whole band.
    pub fn footer_box(self, surface: crate::LayoutBox) -> crate::LayoutBox {
        let band_top = surface.y + surface.height - self.footer_height;
        let (y, height) = if self.drawer {
            (band_top, self.footer_height)
        } else {
            (band_top + self.footer_top, MODAL_ACTION_HEIGHT)
        };
        crate::LayoutBox {
            x: surface.x + self.footer_x,
            y,
            width: (surface.width - self.footer_x * 2.0).max(0.0),
            height,
        }
    }

    /// The top of the footer's actions.
    pub fn action_y(self, surface: crate::LayoutBox) -> f32 {
        surface.y + surface.height - self.footer_height + self.action_top
    }

    /// The hairline under the header, when the recipe draws one.
    pub fn header_divider_box(self, surface: crate::LayoutBox) -> Option<crate::LayoutBox> {
        (self.header_divider > 0.0).then_some(crate::LayoutBox {
            x: surface.x,
            y: surface.y + self.header_height - self.header_divider,
            width: surface.width,
            height: self.header_divider,
        })
    }

    /// The hairline over the footer, when the recipe draws one.
    pub fn footer_divider_box(self, surface: crate::LayoutBox) -> Option<crate::LayoutBox> {
        (self.footer_divider > 0.0).then_some(crate::LayoutBox {
            x: surface.x,
            y: surface.y + surface.height - self.footer_height,
            width: surface.width,
            height: self.footer_divider,
        })
    }
}

pub(crate) fn drawer_width(viewport_width: f32) -> f32 {
    DRAWER_WIDTH.min(viewport_width * 0.92)
}

/// Where a modal surface sits in its scrim `bounds`. A dialog card is as
/// wide as its size asks and stands where the theme's [`DialogRecipe`] puts
/// it; both give way to the scrim's margin, so the card stays inside it.
///
/// [`DialogRecipe`]: nana_ui_core::DialogRecipe
pub(crate) fn modal_surface_bounds(
    bounds: crate::LayoutBox,
    kind: ModalSurfaceKind,
    intrinsic_height: Option<f32>,
    recipe: &nana_ui_core::DialogRecipe,
) -> crate::LayoutBox {
    let margin = 16.0_f32.min(bounds.width / 2.0).min(bounds.height / 2.0);
    let available_width = (bounds.width - margin * 2.0).max(0.0);
    let available_height = (bounds.height - margin * 2.0).max(0.0);
    match kind {
        ModalSurfaceKind::Dialog(size) | ModalSurfaceKind::Confirm(size) => {
            let width = size
                .width_in(bounds.width, bounds.height)
                .min(available_width);
            let max_height = recipe
                .max_height_in(bounds.width, bounds.height)
                .min(available_height);
            let height = intrinsic_height.unwrap_or(max_height).min(max_height);
            let top = recipe
                .top_in(bounds.width, bounds.height)
                .min((bounds.height - margin - height).max(0.0))
                .max(margin);
            crate::LayoutBox {
                x: bounds.x + (bounds.width - width) / 2.0,
                y: bounds.y + top,
                width,
                height,
            }
        }
        ModalSurfaceKind::Drawer(DrawerSide::Left) => {
            let width = drawer_width(bounds.width);
            crate::LayoutBox {
                x: bounds.x,
                y: bounds.y,
                width,
                height: bounds.height,
            }
        }
        ModalSurfaceKind::Drawer(DrawerSide::Right) => {
            let width = drawer_width(bounds.width);
            crate::LayoutBox {
                x: bounds.x + bounds.width - width,
                y: bounds.y,
                width,
                height: bounds.height,
            }
        }
        ModalSurfaceKind::Drawer(DrawerSide::Bottom) => {
            let height = (bounds.height * 0.55).min(520.0).min(bounds.height);
            crate::LayoutBox {
                x: bounds.x,
                y: bounds.y + bounds.height - height,
                width: bounds.width,
                height,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in dialog card's header and body insets.
    const HEADER: nana_ui_core::DialogInsets = nana_ui_core::DialogRecipe::DEFAULT.header;
    const BODY: nana_ui_core::DialogInsets = nana_ui_core::DialogRecipe::DEFAULT.body;
    use crate::{AppContext, Button, DocumentId, MountState, StandardVisual};
    use std::sync::{Arc, Mutex};
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
            crate::TextMetrics {
                width: constraints
                    .max_width
                    .map_or(natural_width, |width| natural_width.min(width)),
                height: count.div_ceil(columns) as f32 * style.font_size * 1.2,
                ascent: None,
            }
        }
    }

    #[test]
    fn modal_slots_replace_by_parking_and_remounting_owned_children() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let confirm = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();

        let first = ModalSlots {
            actions: vec![cancel.stable_id()],
            ..Default::default()
        };
        assert!(cx.set_modal_slots(dialog, first).unwrap());
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Mounted)
        );

        let second = ModalSlots {
            actions: vec![confirm.stable_id()],
            ..Default::default()
        };
        assert!(cx.set_modal_slots(dialog, second).unwrap());
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Parked)
        );
        assert_eq!(
            cx.world().mount_state(confirm.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn modal_slot_failure_is_atomic_across_documents() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let foreign_document = DocumentId::new(2).unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Inspector"))
            .unwrap();
        let current = cx
            .create_detached_component(document, Button::new("Done"))
            .unwrap();
        let foreign = cx
            .create_detached_component(foreign_document, Button::new("Foreign"))
            .unwrap();
        let slots = ModalSlots {
            actions: vec![current.stable_id()],
            ..Default::default()
        };
        cx.set_modal_slots(drawer, slots.clone()).unwrap();

        let error = cx
            .set_modal_slots(
                drawer,
                ModalSlots {
                    actions: vec![foreign.stable_id()],
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::FrameworkError::InvalidModalSlots { .. }
        ));
        assert_eq!(
            cx.read(drawer, |drawer| drawer.slots.clone()).unwrap(),
            slots
        );
        assert_eq!(
            cx.world().mount_state(current.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn confirm_slot_validation_failure_preserves_typed_and_retained_authority() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let foreign_document = DocumentId::new(2).unwrap();
        let dialog = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let confirm = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();
        let foreign = cx
            .create_detached_component(foreign_document, Button::new("Foreign"))
            .unwrap();
        let slots = ConfirmSlots {
            title_icon: None,
            body: None,
            close_action: None,
            cancel: cancel.stable_id(),
            secondary: None,
            confirm: confirm.stable_id(),
        };
        cx.set_confirm_slots(dialog, slots.clone()).unwrap();
        let children = cx.world().node(dialog.stable_id()).unwrap().children;

        assert!(matches!(
            cx.set_confirm_slots(
                dialog,
                ConfirmSlots {
                    cancel: foreign.stable_id(),
                    ..slots.clone()
                }
            ),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.read(dialog, |dialog| dialog.confirm_slots().cloned())
                .unwrap(),
            Some(slots)
        );
        assert_eq!(
            cx.world().node(dialog.stable_id()).unwrap().children,
            children
        );
        assert_eq!(
            cx.world().mount_state(cancel.stable_id()),
            Some(MountState::Mounted)
        );
        assert_eq!(
            cx.world().mount_state(confirm.stable_id()),
            Some(MountState::Mounted)
        );
    }

    #[test]
    fn modal_slots_reject_mounted_roots_and_activation_rejects_rogue_children() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Inspector"))
            .unwrap();
        let mounted_root = cx
            .create_component(document, Button::new("Owned elsewhere"))
            .unwrap();
        assert!(matches!(
            cx.set_modal_slots(
                drawer,
                ModalSlots {
                    body: Some(mounted_root.stable_id()),
                    ..Default::default()
                }
            ),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.world().mount_state(mounted_root.stable_id()),
            Some(MountState::Mounted)
        );
        let rogue = cx
            .create_detached_component(document, Button::new("Rogue"))
            .unwrap();
        cx.append_child(drawer, rogue).unwrap();
        cx.append_child(host, drawer).unwrap();
        assert!(matches!(
            cx.activate_overlay(host, drawer),
            Err(crate::FrameworkError::InvalidModalSlots { .. })
        ));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
    }

    #[test]
    fn bottom_drawer_projects_full_scrim_and_bottom_surface() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("Console").side(DrawerSide::Bottom))
            .unwrap();
        let body = cx
            .create_detached_component(document, Button::new("Body"))
            .unwrap();
        let body_child = cx
            .create_detached_component(document, Button::new("Nested body action"))
            .unwrap();
        cx.append_child(body, body_child).unwrap();
        let action = cx
            .create_detached_component(document, Button::new("Done"))
            .unwrap();
        cx.set_modal_slots(
            drawer,
            ModalSlots {
                body: Some(body.stable_id()),
                actions: vec![action.stable_id()],
                ..Default::default()
            },
        )
        .unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let crate::ComponentGeometry::ModalFrame {
            scrim,
            surface,
            body: body_region,
            ..
        } = cx.world().component_geometry(drawer.stable_id()).unwrap()
        else {
            panic!("modal geometry")
        };
        assert_eq!((scrim.width, scrim.height), (800.0, 600.0));
        assert_eq!(surface.width, 800.0);
        assert_eq!(surface.y + surface.height, 600.0);
        let body_bounds = cx.world().canonical_layout_box(body.stable_id()).unwrap();
        let action_bounds = cx.world().canonical_layout_box(action.stable_id()).unwrap();
        assert!(surface.contains(body_bounds.x, body_bounds.y));
        assert!(surface.contains(action_bounds.x, action_bounds.y));
        assert!(body_bounds.y + body_bounds.height <= action_bounds.y);
        let root_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == drawer.stable_id())
            .unwrap();
        assert_eq!(root_a11y.bounds, surface);

        let escaped_body = crate::LayoutBox {
            x: body_region.x,
            y: body_region.y - 8.0,
            width: 24.0,
            height: 16.0,
        };
        let mut layout = MutationQueue::new();
        layout.write_layout(body.stable_id(), escaped_body);
        cx.commit_mutations(layout).unwrap();
        cx.rebuild_hit_test(document);
        assert!(
            !cx.world()
                .hit_test_candidates(document, escaped_body.x + 1.0, escaped_body.y + 1.0)
                .contains(&body.stable_id())
        );
        assert!(
            cx.world()
                .hit_test_candidates(document, escaped_body.x + 1.0, body_region.y + 1.0)
                .contains(&body.stable_id())
        );

        let translated_body = crate::LayoutBox {
            x: body_region.x,
            y: body_region.y + body_region.height - 8.0,
            width: 24.0,
            height: 8.0,
        };
        let mut translated_style = cx.world().node_style(body.stable_id()).unwrap().clone();
        Arc::make_mut(&mut translated_style.layout).transform =
            Some(nana_ui_core::PaintTransform {
                f: 20.0,
                ..nana_ui_core::PaintTransform::default()
            });
        let mut translated = MutationQueue::new();
        translated.write_layout(body.stable_id(), translated_body);
        translated.write_layout(body_child.stable_id(), translated_body);
        translated.set_style(body.stable_id(), translated_style);
        cx.commit_mutations(translated).unwrap();
        cx.rebuild_hit_test(document);
        let footer_point = (
            translated_body.x + 1.0,
            translated_body.y + translated_body.height / 2.0 + 20.0,
        );
        let footer_candidates =
            cx.world()
                .hit_test_candidates(document, footer_point.0, footer_point.1);
        assert!(!footer_candidates.contains(&body.stable_id()));
        assert!(!footer_candidates.contains(&body_child.stable_id()));
    }

    #[test]
    fn side_drawers_anchor_to_the_requested_viewport_edge() {
        for (index, side) in [DrawerSide::Left, DrawerSide::Right]
            .into_iter()
            .enumerate()
        {
            let mut cx = AppContext::new();
            let document = DocumentId::new(index as u64 + 1).unwrap();
            let drawer = cx
                .create_component(document, Drawer::new("Inspector").side(side))
                .unwrap();
            cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
                .unwrap();
            let crate::ComponentGeometry::ModalFrame { surface, .. } =
                cx.world().component_geometry(drawer.stable_id()).unwrap()
            else {
                panic!("drawer geometry")
            };
            assert_eq!(surface.width, DRAWER_WIDTH);
            assert_eq!(surface.height, 600.0);
            assert_eq!(
                surface.x,
                if side == DrawerSide::Left {
                    0.0
                } else {
                    800.0 - DRAWER_WIDTH
                }
            );
        }
    }

    #[test]
    fn dialog_surface_stays_inside_a_compact_scrim() {
        let scrim = crate::LayoutBox {
            x: 100.0,
            y: 100.0,
            width: 100.0,
            height: 100.0,
        };
        let surface = modal_surface_bounds(
            scrim,
            ModalSurfaceKind::Dialog(DialogSize::Default),
            Some(46.0),
            &nana_ui_core::DialogRecipe::DEFAULT,
        );
        assert!(surface.x >= scrim.x);
        assert!(surface.y >= scrim.y);
        assert!(surface.x + surface.width <= scrim.x + scrim.width);
        assert!(surface.y + surface.height <= scrim.y + scrim.height);
        assert!(surface.contains(150.0, 150.0));
    }

    fn dialog_frame(cx: &AppContext, dialog: StableNodeId) -> (crate::LayoutBox, crate::LayoutBox) {
        let crate::ComponentGeometry::ModalFrame { surface, body, .. } =
            cx.world().component_geometry(dialog).unwrap()
        else {
            panic!("dialog geometry")
        };
        (surface, body)
    }

    /// A dialog names its own width with CSS semantics (`min(560px, 92vw)`
    /// gives way to a narrow window), and the theme's dialog recipe stands
    /// the card 12vh from the top and stops it at 72vh. Installing such a
    /// theme moves a dialog that is already laid out, slots and all.
    #[test]
    fn a_dialog_width_of_its_own_and_the_theme_recipe_place_the_card() {
        use nana_ui_core::{LengthSpec, ViewportAxis};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        // Nested the way an application nests it: under a host, under its page.
        let page = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let host = cx
            .create_detached_component(document, crate::OverlayHost::new())
            .unwrap();
        cx.append_child(page, host).unwrap();
        let dialog = cx
            .create_detached_component(
                document,
                crate::Dialog::new("Export").size(DialogSize::capped(560.0, 92.0)),
            )
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        let body = cx
            .create_detached_component(
                document,
                crate::Stack::column(0.0).height(LengthSpec::Px(2000.0)),
            )
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(body.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(cx.activate_overlay(host, dialog).unwrap());

        cx.layout_document(document, crate::LayoutViewport::new(1200.0, 800.0))
            .unwrap();
        let (surface, _) = dialog_frame(&cx, dialog.stable_id());
        assert_eq!(surface.width, 560.0);
        assert_eq!(surface.y, 90.0);
        assert!((surface.height - 800.0 * 0.76).abs() < 0.01);

        cx.layout_document(document, crate::LayoutViewport::new(500.0, 800.0))
            .unwrap();
        let (surface, _) = dialog_frame(&cx, dialog.stable_id());
        assert!((surface.width - 460.0).abs() < 0.01, "{surface:?}");
        assert!((surface.x - 20.0).abs() < 0.01, "{surface:?}");

        let recipe = nana_ui_core::DialogRecipe {
            top: LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 12.0,
            },
            max_height: LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 72.0,
            },
            ..nana_ui_core::DialogRecipe::DEFAULT
        };
        // The host's incremental path: lay out only what the install dirtied.
        cx.take_system_work();
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(recipe))
            .unwrap();
        let work = cx.take_system_work();
        cx.layout_document_with_frontier(
            document,
            crate::LayoutViewport::new(500.0, 800.0),
            &work.layout_frontier_seeds,
        )
        .unwrap();
        let (surface, body_region) = dialog_frame(&cx, dialog.stable_id());
        assert!((surface.y - 96.0).abs() < 0.01, "{surface:?}");
        assert!((surface.height - 576.0).abs() < 0.01, "{surface:?}");
        let body_box = cx.world().canonical_layout_box(body.stable_id()).unwrap();
        assert_eq!(
            body_box.y, body_region.y,
            "the body slot moved with the card"
        );
        assert!(body_box.y + body_box.height <= surface.y + surface.height);
    }

    fn modal_geometry(cx: &AppContext, modal: StableNodeId) -> crate::ComponentGeometry {
        cx.world().component_geometry(modal).unwrap()
    }

    fn shaped_layout(cx: &mut AppContext, document: DocumentId, width: f32, height: f32) {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        cx.shape_text(&work.text, &mut shaper).unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(width, height))
            .unwrap();
        cx.shape_text_for_layout(document, &mut shaper).unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(width, height))
            .unwrap();
    }

    /// A danger dialog's title speaks in the theme's danger tone, and so
    /// does a confirm dialog's; a plain one keeps the text colour. An icon
    /// slot leads the header row: it and the close button are centred on
    /// the title block, the title starts past the icon and wraps short of
    /// both, and the close button does not grow the row.
    #[test]
    fn a_danger_dialog_speaks_in_the_danger_tone_and_an_icon_leads_its_header() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("删除资源库").danger(true))
            .unwrap();
        let icon = cx
            .create_detached_component(document, crate::Stack::column(0.0))
            .unwrap();
        let close = cx
            .create_detached_component(
                document,
                crate::IconButton::new(nana_ui_core::Icon::Close, "Close"),
            )
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                title_icon: Some(icon.stable_id()),
                close_action: Some(close.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);

        let palette = cx.world().theme().style_model();
        let danger = palette
            .color(nana_ui_core::SemanticColorRole::Danger)
            .as_rgba_array();
        let text = palette
            .color(nana_ui_core::SemanticColorRole::Text)
            .as_rgba_array();
        let crate::ComponentGeometry::ModalFrame {
            surface,
            body,
            title,
            ..
        } = modal_geometry(&cx, dialog.stable_id())
        else {
            panic!("dialog geometry")
        };
        assert_eq!(title.color, Some(danger));
        let recipe = nana_ui_core::DialogRecipe::DEFAULT;
        let icon_box = cx.world().canonical_layout_box(icon.stable_id()).unwrap();
        let close_box = cx.world().canonical_layout_box(close.stable_id()).unwrap();
        assert_eq!(icon_box.x, surface.x + MODAL_PAD_X);
        assert_eq!(
            (icon_box.width, icon_box.height),
            (recipe.icon_size, recipe.icon_size)
        );
        assert_eq!(
            title.bounds.x,
            icon_box.x + recipe.icon_size + recipe.header_gap
        );
        assert!(
            (title.bounds.width
                - (surface.width
                    - MODAL_PAD_X * 2.0
                    - recipe.icon_size
                    - recipe.header_gap * 2.0
                    - recipe.close_size))
                .abs()
                < 0.01,
            "{title:?}"
        );
        let title_middle = title.bounds.y + title.bounds.height / 2.0;
        for square in [icon_box, close_box] {
            assert!(
                (square.y + square.height / 2.0 - title_middle).abs() < 0.01,
                "{square:?} is centred on {title:?}"
            );
        }
        assert_eq!(
            body.y,
            surface.y + HEADER.top + title.bounds.height + HEADER.bottom + BODY.top,
            "the close button does not grow the header row"
        );

        let plain = cx
            .create_component(document, crate::Dialog::new("重命名"))
            .unwrap();
        let confirm = cx
            .create_component(
                document,
                ConfirmDialog::new("删除", "不能恢复").danger(true),
            )
            .unwrap();
        cx.assemble_confirm_dialog(confirm).unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let crate::ComponentGeometry::ModalFrame { title, .. } =
            modal_geometry(&cx, plain.stable_id())
        else {
            panic!("dialog geometry")
        };
        assert_eq!(title.color, Some(text));
        let crate::ComponentGeometry::ModalFrame { title, .. } =
            modal_geometry(&cx, confirm.stable_id())
        else {
            panic!("confirm geometry")
        };
        assert_eq!(title.color, Some(danger));
    }

    /// A theme whose header row is as tall as its close button states that
    /// least height; the title block is then centred in the taller row.
    #[test]
    fn a_header_min_height_holds_the_row_open() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(
            nana_ui_core::DialogRecipe {
                header_min_height: 40.0,
                ..nana_ui_core::DialogRecipe::DEFAULT
            },
        ))
        .unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("导出"))
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let crate::ComponentGeometry::ModalFrame {
            surface,
            body,
            title,
            ..
        } = modal_geometry(&cx, dialog.stable_id())
        else {
            panic!("dialog geometry")
        };
        assert_eq!(
            body.y,
            surface.y + HEADER.top + 40.0 + HEADER.bottom + BODY.top
        );
        assert!(
            (title.bounds.y - (surface.y + HEADER.top + (40.0 - title.bounds.height) / 2.0)).abs()
                < 0.01,
            "{title:?}"
        );
    }

    /// A theme insets the card's three sections, divides them with
    /// hairlines and rounds the card, the way a design writes them as CSS
    /// `padding`, `border-bottom` / `border-top` and `border-radius`. The
    /// footer's slot and its actions sit in its content box, inside the
    /// divider and the insets, the actions `action_gap` apart.
    #[test]
    fn a_theme_insets_divides_and_rounds_the_dialog_card() {
        use nana_ui_core::{DialogInsets, DialogRecipe, LengthSpec, RadiusTier, SemanticColorRole};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let recipe = DialogRecipe {
            radius: RadiusTier::Xl,
            header: DialogInsets::symmetric(12.0, 14.0),
            body: DialogInsets::symmetric(12.0, 14.0),
            body_bottom_alone: None,
            footer: DialogInsets::symmetric(10.0, 14.0),
            action_gap: 8.0,
            header_divider: Some(SemanticColorRole::BorderSoft),
            footer_divider: Some(SemanticColorRole::BorderSoft),
            ..DialogRecipe::DEFAULT
        };
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(recipe))
            .unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("新建文件夹"))
            .unwrap();
        let body = cx
            .create_detached_component(
                document,
                crate::Stack::column(0.0).height(LengthSpec::Px(40.0)),
            )
            .unwrap();
        let footer = cx
            .create_detached_component(document, crate::Stack::row(8.0))
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(body.stable_id()),
                footer: Some(footer.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        let confirm = cx
            .create_component(document, ConfirmDialog::new("删除", "不能恢复"))
            .unwrap();
        cx.assemble_confirm_dialog(confirm).unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);

        let theme = cx.world().theme();
        let hairline = theme.border().hairline;
        let soft = theme
            .style_model()
            .color(SemanticColorRole::BorderSoft)
            .as_rgba_array();
        let radius_xl = theme.metrics().radius_xl;
        let crate::ComponentGeometry::ModalFrame {
            surface,
            title,
            corner_radius,
            header_divider: Some((header_rule, header_color)),
            footer_divider: Some((footer_rule, footer_color)),
            ..
        } = modal_geometry(&cx, dialog.stable_id())
        else {
            panic!("a divided dialog")
        };
        assert_eq!(corner_radius, radius_xl);
        assert_eq!((header_color, footer_color), (soft, soft));
        let header_bottom = surface.y + 12.0 + title.bounds.height + 12.0;
        assert_eq!(
            (header_rule.x, header_rule.width, header_rule.height),
            (surface.x, surface.width, hairline)
        );
        assert!(
            (header_rule.y - header_bottom).abs() < 0.01,
            "{header_rule:?}"
        );
        let body_box = cx.world().canonical_layout_box(body.stable_id()).unwrap();
        assert_eq!(body_box.x, surface.x + 14.0);
        assert_eq!(body_box.width, surface.width - 28.0);
        assert!(
            (body_box.y - (header_bottom + hairline + 12.0)).abs() < 0.01,
            "{body_box:?}"
        );
        assert!(
            (footer_rule.y - (body_box.y + body_box.height + 12.0)).abs() < 0.01,
            "{footer_rule:?}"
        );
        let footer_box = cx.world().canonical_layout_box(footer.stable_id()).unwrap();
        assert_eq!(footer_box.x, surface.x + 14.0);
        assert!(
            (footer_box.y - (footer_rule.y + hairline + 10.0)).abs() < 0.01,
            "{footer_box:?}"
        );
        assert_eq!(footer_box.height, MODAL_ACTION_HEIGHT);
        assert!(
            (surface.y + surface.height - (footer_box.y + footer_box.height + 10.0)).abs() < 0.01,
            "the card ends at the footer's bottom inset"
        );

        let crate::ComponentGeometry::ModalFrame { surface, .. } =
            modal_geometry(&cx, confirm.stable_id())
        else {
            panic!("confirm geometry")
        };
        let slots = cx
            .read(confirm, |confirm| confirm.confirm_slots().cloned())
            .unwrap()
            .unwrap();
        let cancel = cx.world().canonical_layout_box(slots.cancel).unwrap();
        let accept = cx.world().canonical_layout_box(slots.confirm).unwrap();
        assert!((accept.x + accept.width - (surface.x + surface.width - 14.0)).abs() < 0.01);
        assert!((cancel.x + cancel.width - (accept.x - 8.0)).abs() < 0.01);
        assert_eq!(cancel.y, accept.y);
    }

    /// A divided card's rule is as thick as the theme's hairline, and the
    /// body sits under it: a theme that moves only the hairline moves the
    /// body slot, on the host's incremental path too.
    #[test]
    fn a_theme_that_moves_only_the_hairline_moves_the_body_under_the_rule() {
        use nana_ui_core::{DialogRecipe, LengthSpec, SemanticColorRole, ThemeDefinition};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let divided = DialogRecipe {
            header_divider: Some(SemanticColorRole::BorderSoft),
            ..DialogRecipe::DEFAULT
        };
        cx.set_theme_definition(&ThemeDefinition::NANA_DARK.with_dialog(divided))
            .unwrap();
        // Nested the way an application nests it: under a host, under its page.
        let page = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let host = cx
            .create_detached_component(document, crate::OverlayHost::new())
            .unwrap();
        cx.append_child(page, host).unwrap();
        let dialog = cx
            .create_detached_component(document, crate::Dialog::new("新建文件夹"))
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        let body = cx
            .create_detached_component(
                document,
                crate::Stack::column(0.0).height(LengthSpec::Px(40.0)),
            )
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(body.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(cx.activate_overlay(host, dialog).unwrap());
        let viewport = crate::LayoutViewport::new(800.0, 600.0);
        cx.layout_document(document, viewport).unwrap();
        let hairline = cx.world().theme().border().hairline;
        let before = cx.world().canonical_layout_box(body.stable_id()).unwrap();

        let mut thicker = ThemeDefinition::NANA_DARK.with_dialog(divided);
        thicker.tokens.border.hairline = hairline + 2.0;
        cx.take_system_work();
        cx.set_theme_definition(&thicker).unwrap();
        let work = cx.take_system_work();
        cx.layout_document_with_frontier(document, viewport, &work.layout_frontier_seeds)
            .unwrap();
        let after = cx.world().canonical_layout_box(body.stable_id()).unwrap();
        assert!(
            (after.y - before.y - 2.0).abs() < 0.01,
            "{before:?} -> {after:?}"
        );
    }

    /// The built-in recipe keeps the card as it was: no dividers, the card
    /// radius, and a footer slot as tall as an action.
    #[test]
    fn the_built_in_dialog_card_has_no_dividers() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("重命名"))
            .unwrap();
        let footer = cx
            .create_detached_component(document, crate::Stack::row(8.0))
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                footer: Some(footer.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let crate::ComponentGeometry::ModalFrame {
            surface,
            corner_radius,
            header_divider,
            footer_divider,
            ..
        } = modal_geometry(&cx, dialog.stable_id())
        else {
            panic!("dialog geometry")
        };
        assert_eq!(corner_radius, cx.world().theme().metrics().radius_md);
        assert_eq!((header_divider, footer_divider), (None, None));
        let footer_box = cx.world().canonical_layout_box(footer.stable_id()).unwrap();
        assert_eq!(footer_box.height, MODAL_ACTION_HEIGHT);
        assert_eq!(
            surface.y + surface.height,
            footer_box.y + footer_box.height + nana_ui_core::DialogRecipe::DEFAULT.footer.bottom
        );
    }

    /// The scrim is the theme's: its colour and how much it blurs the
    /// window behind it are effect tokens, and a dialog or drawer paints
    /// the ones installed.
    #[test]
    fn the_scrim_is_the_installed_themes() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("导出"))
            .unwrap();
        let drawer = cx.create_component(document, Drawer::new("筛选")).unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        for modal in [dialog.stable_id(), drawer.stable_id()] {
            let crate::ComponentGeometry::ModalFrame {
                scrim_color,
                scrim_blur,
                ..
            } = modal_geometry(&cx, modal)
            else {
                panic!("modal geometry")
            };
            assert_eq!(scrim_color, [0.0, 0.0, 0.0, 0.45]);
            assert_eq!(scrim_blur, 0.0);
        }

        let css = nana_ui_core::linear_scrim_alpha(0.45);
        let mut effects = nana_ui_core::EffectTokens::DARK;
        effects.modal_scrim = nana_ui_core::SemanticColor::rgba(0.0, 0.0, 0.0, css);
        effects.modal_scrim_blur = 2.0;
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_effects(effects))
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        for modal in [dialog.stable_id(), drawer.stable_id()] {
            let crate::ComponentGeometry::ModalFrame {
                scrim_color,
                scrim_blur,
                ..
            } = modal_geometry(&cx, modal)
            else {
                panic!("modal geometry")
            };
            assert_eq!(scrim_color, [0.0, 0.0, 0.0, css]);
            assert_eq!(scrim_blur, 2.0);
        }
    }

    fn moving_recipe() -> nana_ui_core::DialogRecipe {
        use nana_ui_core::{DialogMotion, DialogTransition, motion::Easing};
        nana_ui_core::DialogRecipe {
            motion: DialogMotion {
                scrim: DialogTransition::new(160, Easing::Linear),
                card_fade: DialogTransition::new(120, Easing::Linear),
                card_move: DialogTransition::new(200, Easing::Linear),
                enter_offset_y: -8.0,
                enter_scale: 0.98,
            },
            ..nana_ui_core::DialogRecipe::DEFAULT
        }
    }

    fn hosted_dialog(
        cx: &mut AppContext,
        document: DocumentId,
    ) -> (
        crate::Entity<crate::OverlayHost>,
        crate::Entity<crate::Dialog>,
        StableNodeId,
    ) {
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let dialog = cx
            .create_detached_component(document, crate::Dialog::new("导出"))
            .unwrap();
        let body = cx
            .create_detached_component(
                document,
                crate::Stack::column(0.0).height(LengthSpec::Px(40.0)),
            )
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(body.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        cx.append_child(host, dialog).unwrap();
        (host, dialog, body.stable_id())
    }

    fn presented(
        cx: &AppContext,
        id: StableNodeId,
        property: crate::AnimatableProperty,
    ) -> Option<crate::MotionValue> {
        cx.world()
            .presentation_applied_value(id, property, cx.world().animation_now())
    }

    /// A dialog comes in the way its theme says: the scrim fades on its own
    /// clock, the card fades on another and moves in from 8px above at 98%
    /// about its own centre, slots and all; the exit plays the same back.
    #[test]
    fn a_dialog_enters_and_leaves_with_its_themes_motion() {
        use crate::{AnimatableProperty, MotionValue};
        use std::time::Duration;
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        cx.set_theme_definition(
            &nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(moving_recipe()),
        )
        .unwrap();
        let (host, dialog, body) = hosted_dialog(&mut cx, document);
        cx.advance_animations(Duration::from_millis(1000));
        assert!(cx.activate_overlay(host, dialog).unwrap());
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let id = dialog.stable_id();
        assert_eq!(cx.world().scrim_presence(id), 0.0);
        assert_eq!(
            presented(&cx, id, AnimatableProperty::Opacity),
            Some(MotionValue::Scalar(0.0))
        );
        let Some(MotionValue::Transform(start)) = presented(&cx, id, AnimatableProperty::Transform)
        else {
            panic!("the card moves in")
        };
        assert_eq!((start.a, start.d, start.f), (0.98, 0.98, -8.0));

        // The body follows the card about the card's centre.
        let crate::ComponentGeometry::ModalFrame {
            surface,
            scrim_opacity,
            ..
        } = modal_geometry(&cx, id)
        else {
            panic!("dialog geometry")
        };
        assert_eq!(scrim_opacity, 0.0, "the scrim starts unseen");
        let logical = cx.world().canonical_layout_box(body).unwrap();
        let shown = cx.world().presentation_input_bounds(body).unwrap();
        let centre = surface.y + surface.height / 2.0;
        let expected_top = centre + (logical.y - centre) * 0.98 - 8.0;
        assert!(
            (shown.y - expected_top).abs() < 0.05,
            "{shown:?} against {expected_top}"
        );

        cx.advance_animations(Duration::from_millis(1080));
        assert!((cx.world().scrim_presence(id) - 0.5).abs() < 0.01);
        let crate::ComponentGeometry::ModalFrame { scrim_opacity, .. } = modal_geometry(&cx, id)
        else {
            panic!("dialog geometry")
        };
        assert!((scrim_opacity - 0.5).abs() < 0.01, "{scrim_opacity}");
        let Some(MotionValue::Scalar(card)) = presented(&cx, id, AnimatableProperty::Opacity)
        else {
            panic!("the card fades in")
        };
        assert!((card - 80.0 / 120.0).abs() < 0.01, "{card}");
        let Some(MotionValue::Transform(midway)) =
            presented(&cx, id, AnimatableProperty::Transform)
        else {
            panic!("the card moves in")
        };
        assert!(
            (midway.f - -8.0 * (1.0 - 80.0 / 200.0)).abs() < 0.01,
            "{midway:?}"
        );

        cx.advance_animations(Duration::from_millis(1200));
        assert_eq!(cx.world().scrim_presence(id), 1.0);
        assert!(cx.world().overlay_host(host.stable_id()).unwrap().active == Some(id));

        assert!(cx.dismiss_overlay(host).unwrap());
        cx.advance_animations(Duration::from_millis(1280));
        assert!((cx.world().scrim_presence(id) - 0.5).abs() < 0.01);
        cx.advance_animations(Duration::from_millis(1400));
        assert_eq!(cx.world().scrim_presence(id), 0.0);
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
    }

    /// With reduced motion a dialog is simply there, and simply gone: every
    /// track ends on the next advance, at the same time it began.
    #[test]
    fn reduced_motion_opens_and_closes_a_dialog_at_once() {
        use std::time::Duration;
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        cx.set_theme_definition(
            &nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(moving_recipe()),
        )
        .unwrap();
        cx.set_reduced_motion(true);
        let (host, dialog, _) = hosted_dialog(&mut cx, document);
        let now = Duration::from_millis(1000);
        cx.advance_animations(now);
        assert!(cx.activate_overlay(host, dialog).unwrap());
        cx.advance_animations(now);
        let id = dialog.stable_id();
        assert_eq!(cx.world().scrim_presence(id), 1.0);
        assert!(!cx.world().surface_closing(id));
        assert!(cx.dismiss_overlay(host).unwrap());
        cx.advance_animations(now);
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
    }

    /// A detached dialog with its body slot placed, not yet under a host.
    fn bodied_dialog(
        cx: &mut AppContext,
        document: DocumentId,
        dialog: crate::Dialog,
    ) -> crate::Entity<crate::Dialog> {
        let dialog = cx.create_detached_component(document, dialog).unwrap();
        let body = cx
            .create_detached_component(document, crate::Stack::column(0.0))
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                body: Some(body.stable_id()),
                ..Default::default()
            },
        )
        .unwrap();
        dialog
    }

    fn shows(
        cx: &AppContext,
        host: crate::Entity<crate::OverlayHost>,
        dialog: crate::Entity<crate::Dialog>,
    ) -> bool {
        cx.world().overlay_host(host.stable_id()).unwrap().active == Some(dialog.stable_id())
            && !cx.world().surface_closed(dialog.stable_id())
    }

    /// `open` is the declarative `activate_overlay` / `dismiss_overlay`: a
    /// dialog declared open opens once it is under its host (even when its
    /// host's own update put it there), closes when it turns false and opens
    /// again when it turns back. Parked, it loses its host, as an overlay
    /// always has, and `open` says so: put back, it stays closed until it is
    /// opened again.
    #[test]
    fn a_dialog_declared_open_opens_under_its_host_and_follows_its_field() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let dialog = bodied_dialog(&mut cx, document, crate::Dialog::new("导出").open(true));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        cx.append_child(host, dialog).unwrap();
        assert!(
            shows(&cx, host, dialog),
            "it opens once it is under its host"
        );

        cx.update_component(dialog, |dialog, _| dialog.open = false)
            .unwrap();
        assert!(!shows(&cx, host, dialog), "and closes when it turns false");
        cx.update_component(dialog, |dialog, _| dialog.open = true)
            .unwrap();
        assert!(shows(&cx, host, dialog), "and opens when it turns back");

        cx.update_component(host, |_, cx| {
            cx.mutations().park_subtree(dialog.stable_id());
        })
        .unwrap();
        assert!(!shows(&cx, host, dialog), "parked, the host lets it go");
        assert!(!cx.read(dialog, |dialog| dialog.open).unwrap());
        cx.update_component(host, |_, cx| {
            cx.mutations()
                .insert(host.stable_id(), dialog.stable_id(), None);
        })
        .unwrap();
        assert!(!shows(&cx, host, dialog), "put back, it stays closed");
        cx.update_component(dialog, |dialog, _| dialog.open = true)
            .unwrap();
        assert!(shows(&cx, host, dialog), "until it is opened again");

        // Put in by its host's own update, it opens once the host is back.
        let late = bodied_dialog(&mut cx, document, crate::Dialog::new("稍后").open(true));
        cx.update_component(host, |_, cx| {
            cx.mutations()
                .insert(host.stable_id(), late.stable_id(), None);
        })
        .unwrap();
        assert!(shows(&cx, host, late));
    }

    /// What the host does on its own reaches `open`: a dialog the host
    /// opened is open, one another overlay replaced or the host closed is
    /// not.
    #[test]
    fn the_hosts_own_opens_and_closes_reach_a_dialogs_open() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let first = bodied_dialog(&mut cx, document, crate::Dialog::new("导出"));
        let second = bodied_dialog(&mut cx, document, crate::Dialog::new("设置"));
        cx.append_child(host, first).unwrap();
        cx.append_child(host, second).unwrap();
        let open = |cx: &AppContext, dialog| {
            cx.read(dialog, |dialog: &crate::Dialog| dialog.open)
                .unwrap()
        };

        assert!(cx.activate_overlay(host, first).unwrap());
        assert!(open(&cx, first));
        assert!(cx.activate_overlay(host, second).unwrap());
        assert!(!open(&cx, first), "replaced");
        assert!(open(&cx, second));
        assert!(
            shows(&cx, host, second),
            "writing it back leaves the host as it is"
        );
        assert!(cx.dismiss_overlay(host).unwrap());
        assert!(!open(&cx, second), "closed by the host");
        assert!(!shows(&cx, host, second));
    }

    #[test]
    fn dialog_wraps_against_final_surface_width_and_settles_at_top_inset() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(
                document,
                crate::Dialog::new("一个很长的设置标题👩‍💻需要根据最终宽度换行")
                    .description("说明文字同样按surface内容宽度换行🙂且不会覆盖正文区域"),
            )
            .unwrap();
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        cx.shape_text(&work.text, &mut shaper).unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(220.0, 600.0))
            .unwrap();
        assert!(cx.shape_text_for_layout(document, &mut shaper).unwrap());
        cx.layout_document(document, crate::LayoutViewport::new(220.0, 600.0))
            .unwrap();
        assert!(!cx.shape_text_for_layout(document, &mut shaper).unwrap());

        let crate::ComponentGeometry::ModalFrame {
            surface,
            title,
            description: Some(description),
            body,
            ..
        } = cx.world().component_geometry(dialog.stable_id()).unwrap()
        else {
            panic!("dialog geometry")
        };
        assert_eq!(surface.y, 90.0);
        assert!(surface.height <= 456.0);
        assert!(title.bounds.height > 14.0 * 1.2);
        assert!(description.bounds.height > 12.0 * 1.2);
        assert!(title.bounds.width <= surface.width - 32.0);
        assert!(description.bounds.width <= surface.width - 32.0);
        assert!(description.bounds.y + description.bounds.height <= body.y);
    }

    #[test]
    fn confirm_message_is_body_copy_below_the_title() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let confirm = cx
            .create_component(
                document,
                ConfirmDialog::new("Delete take", "This cannot be undone."),
            )
            .unwrap();
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        cx.shape_text(&work.text, &mut shaper).unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(560.0, 280.0))
            .unwrap();
        let crate::ComponentGeometry::ModalFrame {
            surface,
            title,
            description,
            body_text: Some(message),
            border,
            ..
        } = cx.world().component_geometry(confirm.stable_id()).unwrap()
        else {
            panic!("confirm geometry")
        };
        assert!(description.is_none());
        assert_eq!(message.font_size, MODAL_BODY_TEXT_SIZE);
        assert!(message.bounds.y >= title.bounds.y + title.bounds.height);
        assert!(surface.width <= 520.0);
        assert_eq!(border, [0.0; 4]);
    }

    #[test]
    fn confirm_close_does_not_change_title_to_body_rhythm() {
        fn message_gap(has_close: bool) -> f32 {
            let mut cx = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let confirm = cx
                .create_component(
                    document,
                    ConfirmDialog::new("Delete take", "This cannot be undone."),
                )
                .unwrap();
            let cancel = cx
                .create_detached_component(document, Button::new("取消"))
                .unwrap();
            let accept = cx
                .create_detached_component(document, Button::new("确认"))
                .unwrap();
            let close = has_close.then(|| {
                cx.create_detached_component(
                    document,
                    crate::IconButton::new(nana_ui_core::Icon::Close, "Close"),
                )
                .unwrap()
            });
            cx.set_confirm_slots(
                confirm,
                ConfirmSlots {
                    title_icon: None,
                    body: None,
                    close_action: close.map(|close| close.stable_id()),
                    cancel: cancel.stable_id(),
                    secondary: None,
                    confirm: accept.stable_id(),
                },
            )
            .unwrap();
            let work = cx.take_system_work();
            cx.resolve_styles(&work.style).unwrap();
            let mut shaper = WrappingShaper;
            cx.shape_text(&work.text, &mut shaper).unwrap();
            cx.layout_document(document, crate::LayoutViewport::new(560.0, 280.0))
                .unwrap();
            let crate::ComponentGeometry::ModalFrame {
                title,
                body_text: Some(message),
                ..
            } = cx.world().component_geometry(confirm.stable_id()).unwrap()
            else {
                panic!("confirm geometry")
            };
            message.bounds.y - (title.bounds.y + title.bounds.height)
        }

        let open = message_gap(true);
        let busy = message_gap(false);
        assert!(
            (open - busy).abs() < 0.01,
            "close slot must not change title-to-body gap: open={open} busy={busy}"
        );
        assert!((open - (HEADER.bottom + BODY.top)).abs() < 0.01);
    }

    #[test]
    fn confirm_initial_focus_and_busy_close_are_one_modal_authority() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let confirm = cx
            .create_component(document, ConfirmDialog::new("Delete", "Cannot be undone"))
            .unwrap();
        let cancel = cx
            .create_detached_component(document, Button::new("Cancel"))
            .unwrap();
        let body = cx
            .create_detached_component(document, Button::new("Body action"))
            .unwrap();
        let close = cx
            .create_detached_component(document, Button::new("Close"))
            .unwrap();
        let commit = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();
        let commit_child = cx
            .create_detached_component(document, Button::new("Delete details"))
            .unwrap();
        cx.append_child(commit, commit_child).unwrap();
        cx.set_confirm_slots(
            confirm,
            ConfirmSlots {
                title_icon: None,
                body: Some(body.stable_id()),
                close_action: Some(close.stable_id()),
                cancel: cancel.stable_id(),
                secondary: None,
                confirm: commit.stable_id(),
            },
        )
        .unwrap();
        let intents = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&intents);
        cx.on(confirm, move |_dialog, intent: &ConfirmIntent, _| {
            captured.lock().unwrap().push(*intent)
        })
        .unwrap();
        cx.append_child(host, confirm).unwrap();
        assert!(cx.activate_overlay(host, confirm).unwrap());
        assert_eq!(cx.world().focused(document), Some(cancel.stable_id()));
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        assert!(cx.activate_button(commit).unwrap());
        assert_eq!(
            *intents.lock().unwrap(),
            vec![ConfirmIntent::Confirm { danger: false }]
        );
        assert!(cx.focus_node(document, commit_child.stable_id()).unwrap());
        cx.take_system_work();
        cx.set_confirm_state(confirm, true, true).unwrap();
        assert_eq!(cx.world().focused(document), Some(confirm.stable_id()));
        let busy_work = cx.take_system_work();
        let busy_delta = cx.world().project_accessibility_delta(&busy_work);
        assert!(
            busy_delta
                .updated
                .iter()
                .any(|node| node.id == commit.stable_id() && node.disabled)
        );
        cx.restore_system_work(busy_work);
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        assert!(!cx.activate_button(close).unwrap());
        assert!(!cx.activate_button(commit).unwrap());
        assert!(!cx.focus_node(document, cancel.stable_id()).unwrap());
        assert!(
            !cx.apply_accessibility_action(
                document,
                crate::AccessibilityActionRequest {
                    target: commit_child.stable_id(),
                    action: crate::AccessibilityAction::Focus,
                },
            )
            .unwrap()
        );
        assert!(cx.focus_node(document, body.stable_id()).unwrap());
        assert!(cx.focus_node(document, confirm.stable_id()).unwrap());
        let commit_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == commit.stable_id())
            .unwrap();
        assert!(commit_a11y.disabled);
        let extracted = cx
            .world()
            .extract_nodes(&[commit.stable_id()])
            .pop()
            .unwrap();
        assert!(matches!(
            extracted.standard_visual,
            Some(StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Danger,
                loading: true,
                ..
            })
        ));
        cx.rebuild_hit_test(document);
        let commit_bounds = cx.world().canonical_layout_box(commit.stable_id()).unwrap();
        let commit_center = (
            commit_bounds.x + commit_bounds.width / 2.0,
            commit_bounds.y + commit_bounds.height / 2.0,
        );
        assert!(
            !cx.world()
                .hit_test_candidates(document, commit_center.0, commit_center.1)
                .contains(&commit.stable_id())
        );
        assert!(
            cx.world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active
                .is_some()
        );
        assert!(
            cx.route_overlay_key(document, crate::OverlayKey::Escape)
                .unwrap()
        );
        assert!(
            cx.world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active
                .is_some()
        );
        cx.set_confirm_state(confirm, false, false).unwrap();
        let restored_work = cx.take_system_work();
        let restored_delta = cx.world().project_accessibility_delta(&restored_work);
        assert!(
            restored_delta
                .updated
                .iter()
                .any(|node| node.id == commit.stable_id() && !node.disabled)
        );
        cx.restore_system_work(restored_work);
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        cx.rebuild_hit_test(document);
        assert!(
            cx.world()
                .hit_test_candidates(document, commit_center.0, commit_center.1)
                .contains(&commit.stable_id())
        );
        let restored_a11y = cx
            .world()
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == commit.stable_id())
            .unwrap();
        assert!(!restored_a11y.disabled);
        let restored = cx
            .world()
            .extract_nodes(&[commit.stable_id()])
            .pop()
            .unwrap();
        assert!(matches!(
            restored.standard_visual,
            Some(StandardVisual::Button {
                kind: nana_ui_core::ButtonKind::Primary,
                loading: false,
                ..
            })
        ));
    }

    #[test]
    fn invalid_explicit_initial_focus_rejects_activation_atomically() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let foreign = cx
            .create_component(document, Button::new("Outside"))
            .unwrap();
        let dialog = cx
            .create_component(
                document,
                Drawer::new("Inspector")
                    .initial_focus(ModalInitialFocus::Target(foreign.stable_id())),
            )
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        let error = cx.activate_overlay(host, dialog).unwrap_err();
        assert!(matches!(
            error,
            crate::FrameworkError::InvalidComponentHierarchy { .. }
        ));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        assert_eq!(cx.world().focused(document), None);
    }
}
