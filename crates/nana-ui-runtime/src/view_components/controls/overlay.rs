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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverlayHost {
    pub style: NodeStyle,
}

impl OverlayHost {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new() -> Self {
        Self::default()
    }
}

impl ComponentView for OverlayHost {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "overlay-host".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.overlay_host(id).is_none() {
            mutations.set_overlay_host(id, OverlayHostState::default());
        }
        // Whether a host takes the pointer follows the active overlay's kind,
        // which activation resolves and writes. Projection would only see the
        // state from before that commit, so it carries the value forward and
        // starts out transparent.
        let interaction = world.interaction(id).unwrap_or(InteractionState {
            pointer_events: false,
            focusable: false,
        });
        project_common(
            id,
            world,
            mutations,
            &self.style,
            interaction,
            AccessibilityState::default(),
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub size: nana_ui_core::DialogSize,
    /// A destructive dialog: the title takes the theme's danger colour, the
    /// tone a [`crate::ConfirmDialog`] with `danger` speaks in.
    pub danger: bool,
    /// Open while it is in a tree under an [`OverlayHost`]: the framework
    /// makes it the host's open overlay as soon as it, its host and its
    /// slots are there, and closes it, exit and all, when this turns false.
    /// It follows the host too: opening it with
    /// [`crate::AppContext::activate_overlay`] sets it; a close the framework
    /// makes, another overlay of the host opening in its place, or parking
    /// it (the host lets go of a parked overlay) clears it.
    pub open: bool,
    pub close_policy: nana_ui_core::DialogClosePolicy,
    pub initial_focus: crate::ModalInitialFocus,
    pub slots: crate::ModalSlots,
    /// Paints the card in place of the theme's rounded card. See
    /// [`Self::surface_painter`](fn@Self::surface_painter).
    pub surface_painter: Option<crate::NodePainter>,
    pub style: NodeStyle,
}

impl Dialog {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        let size = nana_ui_core::DialogSize::default();
        Self {
            title: title.into(),
            description: None,
            size,
            danger: false,
            open: false,
            close_policy: nana_ui_core::DialogClosePolicy::default(),
            initial_focus: crate::ModalInitialFocus::default(),
            slots: crate::ModalSlots::default(),
            surface_painter: None,
            style: crate::overlay_surfaces::modal_root_style(),
        }
    }

    /// Paint the card with `painter` in place of the theme's rounded card:
    /// its fill, edge and shadow, in the card's own coordinates, over the
    /// scrim and under the card's title, text, dividers and slots. The
    /// painter reads where the header, title, body and footer lie with
    /// [`crate::PaintContext::modal_sections`], so a shape that rises around
    /// the header follows it. The theme's recipe still places, divides and
    /// moves the card; only how it looks is the painter's. It is recorded
    /// again only when its key, the card's size or sections, or the theme
    /// change.
    ///
    /// Behind-children paint only: `paint_over_children` and the painter's
    /// hit test are not used, and the whole card takes presses as a modal
    /// card does. `draw_default()` draws the theme's card under the
    /// painter's ops.
    pub fn surface_painter(mut self, painter: impl Into<crate::NodePainter>) -> Self {
        self.surface_painter = Some(painter.into());
        self
    }

    pub fn size(mut self, size: nana_ui_core::DialogSize) -> Self {
        self.size = size;
        self
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Speak in the danger tone: the title takes the theme's danger colour.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Open while it is in a tree under an [`OverlayHost`]; see
    /// [`Self::open`](field@Self::open).
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn close_policy(mut self, policy: nana_ui_core::DialogClosePolicy) -> Self {
        self.close_policy = policy;
        self
    }

    pub fn initial_focus(mut self, initial_focus: crate::ModalInitialFocus) -> Self {
        self.initial_focus = initial_focus;
        self
    }

    pub(crate) fn set_initial_focus(&mut self, initial_focus: crate::ModalInitialFocus) {
        self.initial_focus = initial_focus;
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

pub(crate) const MODAL_OVERLAY_BEHAVIOR: crate::framework::OverlayBehavior =
    crate::framework::OverlayBehavior {
        note_modal_open: Some(AppContext::note_modal_open),
        sync_modal_open: Some(AppContext::sync_modal_open_at),
        announce_close_request: Some(AppContext::announce_close_request),
        announce_confirm_intent: Some(AppContext::announce_confirm_intent),
        note_anchored_menu_dismissed: None,
        dismiss_context_menu: None,
        note_tooltip: None,
        open_hover_card: None,
        close_hover_card: None,
    };

impl ComponentView for Dialog {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DIALOG.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_modal_slots::<Self>),
        lifecycle: Some(crate::AppContext::sync_modal_open::<Self>),
        overlay: Some(&MODAL_OVERLAY_BEHAVIOR),
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
            tag: "dialog".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        crate::overlay_surfaces::project_modal(
            id,
            world,
            mutations,
            &self.style,
            AccessibilityRole::Dialog,
            &self.title,
            self.description.as_deref(),
            None,
            crate::ModalSurfaceKind::Dialog(self.size),
            false,
            self.danger,
            &self.slots,
            self.surface_painter.as_ref(),
        );
    }
}

impl crate::ModalSurface for Dialog {
    fn slots(&self) -> &crate::ModalSlots {
        &self.slots
    }
    fn slots_mut(&mut self) -> &mut crate::ModalSlots {
        &mut self.slots
    }
    fn is_open(&self) -> bool {
        self.open
    }
    fn set_open(&mut self, open: bool) {
        self.open = open;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub label: Arc<str>,
    pub config: nana_ui_core::TooltipConfig,
    pub style: NodeStyle,
}

impl Tooltip {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self::with_config(label, nana_ui_core::TooltipConfig::default())
    }

    pub fn with_config(label: impl Into<Arc<str>>, config: nana_ui_core::TooltipConfig) -> Self {
        let mut style = tooltip_surface_style(config.max_width.max(0.0));
        Arc::make_mut(&mut style.layout).max_width =
            Some(nana_ui_core::LengthSpec::Px(config.max_width.max(0.0)));
        Self {
            label: label.into(),
            config,
            style,
        }
    }

    /// Set where the floating tooltip mounts visually in the scene tree
    /// (defaults to [`nana_ui_core::OverlayMount::NearestModal`]).
    pub fn overlay_mount(mut self, mount: nana_ui_core::OverlayMount) -> Self {
        self.config.overlay_mount = mount;
        self
    }
}

impl ComponentView for Tooltip {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TOOLTIP.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tooltip".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.overlay_mount(id) != Some(self.config.overlay_mount) {
            mutations.set_overlay_mount(id, Some(self.config.overlay_mount));
        }
        if world.text(id) != Some(self.label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.to_string().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState {
                role: AccessibilityRole::Tooltip,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn tooltip_surface_style(max_width: f32) -> NodeStyle {
    let mut theme = nana_ui_core::ThemeLayout {
        font_size: Some(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Hint).into()),
        border_width: Some(
            nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into(),
        ),
        padding_left: Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm)),
        padding_right: Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm)),
        padding_top: Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs)),
        padding_bottom: Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs)),
        ..Default::default()
    };
    theme.border_radius = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
    let mut layout = nana_ui_core::LayoutStyle {
        theme: theme.into(),
        position: nana_ui_core::PositionSpec::Fixed,
        max_width: Some(nana_ui_core::LengthSpec::Px(max_width)),
        padding_left: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::TooltipConfig::PADDING_X,
        )),
        padding_right: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::TooltipConfig::PADDING_X,
        )),
        padding_top: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::TooltipConfig::PADDING_Y,
        )),
        padding_bottom: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::TooltipConfig::PADDING_Y,
        )),
        border_width: Some(nana_ui_core::HAIRLINE),
        border_radius: Some(nana_ui_core::TooltipConfig::RADIUS),
        font_size: Some(nana_ui_core::TooltipConfig::FONT_SIZE),
        z_index: Some(1_000),
        ..nana_ui_core::LayoutStyle::default()
    };
    layout.text_ext.overflow_wrap = Some(nana_ui_core::OverflowWrapSpec::Anywhere);
    NodeStyle {
        layout: Arc::new(layout),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderSoft),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        ..NodeStyle::default()
    }
}
