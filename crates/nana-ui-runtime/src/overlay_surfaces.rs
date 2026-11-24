use std::sync::Arc;

use nana_ui_core::{
    ControlSize, DialogClosePolicy, DialogSize, DrawerSide, LayoutStyle, LengthSpec, PositionSpec,
};

use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, UiWorld, view_components::project_common,
};

/// Open visual contract attached to a modal frame (`Dialog`, `ConfirmDialog`, `Drawer`).
#[derive(Debug, Clone, PartialEq)]
pub struct ModalFrameVisual {
    pub title: Arc<str>,
    pub description: Option<Arc<str>>,
    pub body_text: Option<Arc<str>>,
    pub kind: ModalSurfaceKind,
    pub busy: bool,
    pub danger: bool,
    pub slots: ModalSlots,
    /// Paints the card in place of the theme's; see
    /// [`crate::Dialog::surface_painter`].
    pub surface_painter: Option<crate::NodePainter>,
}

/// Page-space or local geometry projected by [`ModalFrameVisual`].
#[derive(Debug, Clone, PartialEq)]
pub struct ModalFrameGeometry {
    pub scrim: crate::LayoutBox,
    pub surface: crate::LayoutBox,
    pub body: crate::LayoutBox,
    pub title: crate::ComponentTextRegion,
    pub description: Option<crate::ComponentTextRegion>,
    pub body_text: Option<crate::ComponentTextRegion>,
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub elevation: crate::ComponentElevation,
    /// The card's corner radius: a dialog's recipe step, resolved.
    pub corner_radius: f32,
    /// The hairline under the header and its colour, when the recipe
    /// draws one.
    pub header_divider: Option<(crate::LayoutBox, [f32; 4])>,
    /// The hairline over the footer and its colour, when the recipe
    /// draws one.
    pub footer_divider: Option<(crate::LayoutBox, [f32; 4])>,
    /// The scrim's colour: the theme's modal scrim.
    pub scrim_color: [f32; 4],
    /// How much the scrim blurs what is behind it: CSS `blur(r)`, `r`
    /// the standard deviation in logical px.
    pub scrim_blur: f32,
    /// How much of the scrim shows, 0 to 1: CSS `opacity` on the
    /// overlay. It fades the scrim's colour and its blur together while
    /// the frame comes and goes; 1 at rest.
    pub scrim_opacity: f32,
    /// The card as its surface painter recorded it, in the card's own
    /// coordinates; `None` paints the theme's card.
    pub surface_paint: Option<Arc<crate::PaintRecording>>,
}

impl ModalFrameGeometry {
    /// Transform pivot offset relative to `bounds` origin (centre of the card).
    pub fn presentation_pivot(&self, bounds: crate::LayoutBox) -> [f32; 2] {
        [
            self.surface.x + self.surface.width / 2.0 - bounds.x,
            self.surface.y + self.surface.height / 2.0 - bounds.y,
        ]
    }
}

impl ModalFrameVisual {
    /// Derives [`ModalFrameGeometry`] for `id` in `bounds`.
    pub fn geometry_for(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
        style: &crate::ComputedStyle,
        style_model: nana_ui_core::StyleModelRef,
        theme: &nana_ui_core::CompiledTheme,
    ) -> Option<ModalFrameGeometry> {
        let crate::world::content_parts::ModalFrameLayout {
            chrome,
            surface,
            title_content,
            ..
        } = world.modal_frame_layout(id, bounds)?;
        let secondary = world.secondary_text_color(id);
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content: content.into(),
                color: Some(if muted {
                    secondary
                } else {
                    style.color.unwrap_or_else(|| {
                        style_model
                            .color(nana_ui_core::SemanticColorRole::Text)
                            .as_rgba_array()
                    })
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        let recipe = theme.recipes().dialog();
        let parts = world.component_content_parts(id);
        let part = |kind| crate::layout_engine::parts::part_bounds(&parts, kind);
        let title_box = part(crate::LayoutPartKind::Header).unwrap_or(crate::LayoutBox {
            x: surface.x,
            y: surface.y,
            ..crate::LayoutBox::default()
        });
        let description_box = part(crate::LayoutPartKind::Detail);
        let title_type = modal_title_type(self.kind, recipe, theme.typography());
        let body = chrome.body_box(surface);
        let surface_paint = self.surface_painter.as_ref().map(|painter| {
            let local = |section: crate::LayoutBox| crate::LayoutBox {
                x: section.x - surface.x,
                y: section.y - surface.y,
                ..section
            };
            // The header band stops where its divider starts, so a
            // shape drawn to its height meets the rule, not its far edge.
            let header_height = chrome
                .header_divider_box(surface)
                .map_or(chrome.header_height, |rule| rule.y - surface.y);
            let sections = crate::ModalSurfaceSections {
                header: crate::LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: surface.width,
                    height: header_height,
                },
                // The title and the description under it.
                title: local(crate::LayoutBox {
                    height: description_box.map_or(title_box.height, |description| {
                        description.y + description.height - title_box.y
                    }),
                    ..title_box
                }),
                icon: chrome.icon_box(surface).map(local),
                close: chrome.close_box(surface).map(local),
                body: local(body),
                footer: (chrome.footer_height > 0.0).then_some(crate::LayoutBox {
                    x: 0.0,
                    y: surface.height - chrome.footer_height,
                    width: surface.width,
                    height: chrome.footer_height,
                }),
            };
            world.modal_surface_paint(id, painter, surface, sections)
        });
        // A danger dialog speaks in the status recipe's danger tone,
        // over whatever colour the node's text would take, unless
        // the recipe marks danger some other way.
        let title_color = if self.danger && recipe.danger_title {
            style_model
                .color(
                    theme
                        .recipes()
                        .status()
                        .role(nana_ui_core::StatusTone::Danger),
                )
                .as_rgba_array()
        } else {
            style.color.unwrap_or_else(|| {
                style_model
                    .color(
                        theme
                            .recipes()
                            .foreground(nana_ui_core::ComponentRecipeId::Overlay, false),
                    )
                    .as_rgba_array()
            })
        };
        let overlay_shadow = theme.shadow(nana_ui_core::ElevationRole::Overlay);
        // The scrim rides the card's move on the frame's own layer,
        // so it reaches past the frame by as much as the move and
        // the entrance scale could pull its edge in: an even colour
        // that still covers the window wherever the card is.
        let motion = recipe.motion;
        let scrim = match self.kind {
            ModalSurfaceKind::Drawer(_) => bounds,
            _ if motion.moves() => {
                let scale = motion.enter_scale.max(f32::EPSILON);
                let reach = bounds.width.max(bounds.height) * (1.0 / scale - 1.0).max(0.0)
                    + motion.enter_offset_y.abs() / scale
                    + 1.0;
                crate::LayoutBox {
                    x: bounds.x - reach,
                    y: bounds.y - reach,
                    width: bounds.width + reach * 2.0,
                    height: bounds.height + reach * 2.0,
                }
            }
            _ => bounds,
        };
        Some(ModalFrameGeometry {
            scrim,
            surface,
            body,
            title: crate::ComponentTextRegion {
                color: Some(title_color),
                ..text_region(
                    title_box,
                    if title_content.is_some() {
                        Arc::from("")
                    } else {
                        Arc::clone(&self.title)
                    },
                    false,
                    title_type.size,
                    title_type.weight,
                )
            },
            description: self.description.as_ref().zip(description_box).map(
                |(description, bounds)| {
                    text_region(
                        bounds,
                        Arc::clone(description),
                        true,
                        theme.typography().meta,
                        None,
                    )
                },
            ),
            body_text: self
                .body_text
                .as_ref()
                .zip(part(crate::LayoutPartKind::Note))
                .map(|(message, bounds)| {
                    text_region(
                        bounds,
                        Arc::clone(message),
                        false,
                        theme.typography().body,
                        None,
                    )
                }),
            background: style_model
                .color(nana_ui_core::SemanticColorRole::Surface)
                .as_rgba_array(),
            border: [0.0; 4],
            elevation: crate::ComponentElevation::from_shadow(overlay_shadow),
            corner_radius: match self.kind {
                ModalSurfaceKind::Drawer(_) => style_model.metrics.radius_md,
                _ => recipe.radius.resolve(style_model.metrics),
            },
            header_divider: chrome.header_divider_box(surface).zip(
                recipe
                    .header_divider
                    .map(|role| style_model.color(role).as_rgba_array()),
            ),
            footer_divider: chrome.footer_divider_box(surface).zip(
                recipe
                    .footer_divider
                    .map(|role| style_model.color(role).as_rgba_array()),
            ),
            scrim_color: theme.effects().modal_scrim.as_rgba_array(),
            scrim_blur: theme.effects().modal_scrim_blur,
            surface_paint,
            scrim_opacity: world.scrim_presence(id),
        })
    }

    /// Emits node-local [`crate::WidgetPaintOutput`] operations from precomputed `geometry`.
    pub fn paint_with_geometry(
        &self,
        geometry: &ModalFrameGeometry,
        bounds: crate::LayoutBox,
    ) -> crate::WidgetPaintOutput {
        let mut output = crate::WidgetPaintOutput {
            owns_node_text: true,
            presentation_pivot: Some(geometry.presentation_pivot(bounds)),
            modal_frame: Some(crate::WidgetModalFrame {
                surface: geometry.surface,
                body: geometry.body,
                body_root: self.slots.body,
                scrim_slot: Some(crate::MODAL_SCRIM_SLOT),
            }),
            ..Default::default()
        };
        // The scrim fades by its own opacity, not its node's layer (the card's).
        output.push_tagged(
            crate::WidgetTaggedOp::new(
                crate::MODAL_SCRIM_SLOT,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: geometry.scrim,
                    fill: Some(geometry.scrim_color),
                    backdrop_blur: geometry.scrim_blur,
                    ..Default::default()
                }),
            )
            .with_opacity_multiplier(geometry.scrim_opacity.clamp(0.0, 1.0)),
        );
        let docked = match self.kind {
            ModalSurfaceKind::Drawer(side) => Some(side),
            _ => None,
        };
        let radius = geometry.corner_radius;
        let mut surface_bounds = geometry.surface;
        if let Some(side) = docked {
            match side {
                DrawerSide::Right => surface_bounds.width += radius,
                DrawerSide::Left => {
                    surface_bounds.x -= radius;
                    surface_bounds.width += radius;
                }
                DrawerSide::Bottom => surface_bounds.height += radius,
            }
        }
        // A dialog that paints its own card leaves the theme's out, unless
        // its painter asked for it with `draw_default()`.
        let themed_card = geometry
            .surface_paint
            .as_ref()
            .is_none_or(|recording| recording.default_phase().is_some());
        if themed_card {
            let mut op = crate::WidgetTaggedOp::new(
                11,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: surface_bounds,
                    fill: Some(geometry.background),
                    corner_radius: [radius; 4],
                    elevation: Some(geometry.elevation),
                    ..Default::default()
                }),
            );
            if docked.is_some() {
                op = op.with_extra_clip(geometry.scrim);
            }
            output.push_tagged(op);
        }
        if let Some(recording) = &geometry.surface_paint {
            output.push(
                11,
                crate::WidgetPaintOp::Vector {
                    bounds: geometry.surface,
                    recording: Arc::clone(recording),
                },
            );
        }
        // The card's section dividers: hairlines across the whole card, inside
        // its edges, so the card's own clip trims them.
        for (slot, divider) in [(15, geometry.header_divider), (16, geometry.footer_divider)] {
            if let Some((rect, color)) = divider {
                output.push(
                    slot,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect,
                        fill: Some(color),
                        ..Default::default()
                    }),
                );
            }
        }
        let mut emit_text = |slot: u64, region: &crate::ComponentTextRegion| {
            output.push(
                slot,
                crate::WidgetPaintOp::Text(crate::WidgetText {
                    bounds: region.bounds,
                    content: region.content.clone(),
                    color: region.color,
                    font_size: region.font_size,
                    font_weight: region.font_weight,
                    horizontal_alignment: crate::TextHorizontalAlignment::Start,
                    vertical_alignment: crate::TextVerticalAlignment::Top,
                    ellipsis: true,
                    wrap: true,
                    line_height: Some(None),
                    retained_text: crate::WidgetRetainedText::NodeTextIfRich,
                    ..Default::default()
                }),
            );
        };
        if !geometry.title.content.is_empty() {
            emit_text(12, &geometry.title);
        }
        if let Some(description) = &geometry.description {
            emit_text(13, description);
        }
        if let Some(body_text) = &geometry.body_text {
            emit_text(14, body_text);
        }
        output
    }
}

impl crate::WidgetVisual for ModalFrameVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_DIALOG)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn is_confirm_modal(&self) -> bool {
        matches!(self.kind, ModalSurfaceKind::Confirm(_))
    }

    fn is_modal_frame(&self) -> bool {
        true
    }

    fn owns_intrinsic_text(&self) -> bool {
        true
    }

    fn text_visual_key(&self) -> Option<(std::any::TypeId, u64)> {
        Some((std::any::TypeId::of::<Self>(), 0))
    }

    fn modal_slots(&self) -> Option<ModalSlots> {
        Some(self.slots.clone())
    }

    fn modal_layout_input(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> Option<crate::ModalLayoutInput> {
        let presentation = world.modal_text_metrics(id);
        Some(crate::ModalLayoutInput {
            kind: self.kind,
            slots: self.slots.clone(),
            title: presentation.title,
            description: presentation.description,
            body_text: presentation.body,
        })
    }

    fn paint_depends_on_child(&self, child: StableNodeId) -> bool {
        self.slots.body == Some(child)
    }

    fn presentation_pivot(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
    ) -> Option<[f32; 2]> {
        let frame = world.modal_frame_layout(id, bounds)?;
        Some([
            frame.surface.x + frame.surface.width / 2.0 - bounds.x,
            frame.surface.y + frame.surface.height / 2.0 - bounds.y,
        ])
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
        _content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        Some(world.modal_text_parts(id, bounds))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let Some(geometry) = self.geometry_for(
            ctx.id,
            ctx.world,
            ctx.bounds,
            ctx.style,
            ctx.style_model,
            ctx.theme,
        ) else {
            return crate::WidgetPaintOutput::default();
        };
        self.paint_with_geometry(&geometry, ctx.bounds)
    }
}

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
    /// What the header row shows in place of the title text: a view laid
    /// out like any other, such as a name followed by a status badge. It
    /// stands where the title block stands, after the icon and before the
    /// close button, as wide as the title may run and as tall as its own
    /// content, centred in the row. The surface's `title` stays its
    /// accessible name and is no longer drawn. A slot that generates no box
    /// (`display: none`) gives the row back to the title text.
    pub title_content: Option<StableNodeId>,
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
    /// Content at the start of the action row, one action tall: a "don't
    /// ask again" switch beside the buttons.
    pub footer: Option<StableNodeId>,
    pub close_action: Option<StableNodeId>,
    pub cancel: StableNodeId,
    pub secondary: Option<StableNodeId>,
    pub confirm: StableNodeId,
}

impl ConfirmSlots {
    pub(crate) fn modal_slots(&self) -> ModalSlots {
        ModalSlots {
            title_icon: self.title_icon,
            title_content: None,
            body: self.body,
            footer: self.footer,
            close_action: self.close_action,
            actions: std::iter::once(self.cancel)
                .chain(self.secondary)
                .chain([self.confirm])
                .collect(),
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
            .chain(self.title_content)
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

/// Where a modal dialog card's sections lie, in the card's own coordinates
/// (origin at its top-left, logical px): what a card surface painter
/// ([`crate::Dialog::surface_painter`], [`ConfirmDialog::surface_painter`])
/// reads through [`crate::PaintContext::modal_sections`] to shape the card
/// around its header, body and footer. The boxes are the ones the card lays
/// its text and slots in, so a shape drawn on them stays under what they
/// hold.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModalSurfaceSections {
    /// The header band, the card's full width from its top: the header row
    /// with its insets, down to the header divider when the recipe draws
    /// one (the divider itself lies just under the band).
    pub header: crate::LayoutBox,
    /// The title block: the title and its description, as wide as the
    /// recipe lets them run. With a title content slot
    /// ([`ModalSlots::title_content`]) it is that slot's box over the
    /// description.
    pub title: crate::LayoutBox,
    /// The title icon's square, when the dialog has one.
    pub icon: Option<crate::LayoutBox>,
    /// The close button's square, when the dialog has one.
    pub close: Option<crate::LayoutBox>,
    /// The body's content box.
    pub body: crate::LayoutBox,
    /// The footer band, the card's full width down to its bottom, when the
    /// dialog has actions or a footer slot.
    pub footer: Option<crate::LayoutBox>,
}

impl ModalSurfaceSections {
    /// A digest of every box, for the recording cache: two sections that
    /// lie the same digest the same.
    pub(crate) fn digest(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut write = |section: Option<crate::LayoutBox>| match section {
            Some(section) => {
                1u8.hash(&mut hasher);
                for value in [section.x, section.y, section.width, section.height] {
                    value.to_bits().hash(&mut hasher);
                }
            }
            None => 0u8.hash(&mut hasher),
        };
        write(Some(self.header));
        write(Some(self.title));
        write(self.icon);
        write(self.close);
        write(Some(self.body));
        write(self.footer);
        hasher.finish()
    }
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
    /// Paints the card in place of the theme's; see
    /// [`crate::Dialog::surface_painter`].
    pub surface_painter: Option<crate::NodePainter>,
    pub style: NodeStyle,
}

/// What a dialog was given before its first assembly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RequestedConfirmSlots {
    pub title_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub footer: Option<StableNodeId>,
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
            surface_painter: None,
            style: modal_root_style(),
        }
    }

    /// Paint the card with `painter` in place of the theme's rounded card;
    /// see [`crate::Dialog::surface_painter`].
    pub fn surface_painter(mut self, painter: impl Into<crate::NodePainter>) -> Self {
        self.surface_painter = Some(painter.into());
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

    /// Content at the start of the action row, placed on assembly: one
    /// action tall, as wide as the row leaves beside the actions.
    pub fn footer(mut self, footer: StableNodeId) -> Self {
        self.requested.footer = Some(footer);
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
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::CONFIRM_DIALOG.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_confirm_dialog),
        lifecycle: Some(crate::AppContext::sync_modal_open::<Self>),
        overlay: Some(&crate::view_components::MODAL_OVERLAY_BEHAVIOR),
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
            self.surface_painter.as_ref(),
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
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DRAWER.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_modal_slots::<Self>),
        lifecycle: Some(crate::AppContext::sync_modal_open::<Self>),
        overlay: Some(&crate::view_components::MODAL_OVERLAY_BEHAVIOR),
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
            None,
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
    surface_painter: Option<&crate::NodePainter>,
) {
    let visual = ModalFrameVisual {
        title: Arc::clone(title),
        description: description.map(Arc::from),
        body_text: body_text.map(Arc::from),
        kind,
        busy,
        danger,
        slots: slots.clone(),
        surface_painter: surface_painter.cloned(),
    };
    if world.widget_visual_as::<ModalFrameVisual>(id) != Some(&visual) {
        mutations.set_widget_visual(id, Some(visual));
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
    .theme(
        nana_ui_core::ThemeProperty::FontSize,
        nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Body),
    )
}

pub(crate) const DRAWER_WIDTH: f32 = 360.0;
pub(crate) const MODAL_PAD_X: f32 = nana_ui_core::space::XXXL;
#[cfg(test)]
pub(crate) const MODAL_ACTION_HEIGHT: f32 = ControlSize::Medium.height_in(nana_ui_core::UI_METRICS);

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
    action_height: f32,
    header_divider: f32,
    footer_divider: f32,
    /// Whether the hairlines keep inside the header's and the footer's
    /// inline insets rather than running edge to edge.
    inset_dividers: bool,
    /// The recipe's widest header text, unresolved: it reads the card and
    /// the scrim, which the measure does not know yet.
    title_max_width: Option<LengthSpec>,
    drawer: bool,
}

impl ModalChrome {
    /// `hairline` is the installed theme's stroke, the thickness of a
    /// divider the recipe asks for.
    pub fn measure_in(
        kind: ModalSurfaceKind,
        title: crate::TextMetrics,
        description: Option<crate::TextMetrics>,
        slots: &ModalSlots,
        recipe: &nana_ui_core::DialogRecipe,
        hairline: f32,
        theme: &nana_ui_core::CompiledTheme,
    ) -> Self {
        let spacing = theme.spacing();
        let action_height = ControlSize::Medium.height_in(theme.metrics());
        let has_footer = slots.footer.is_some() || !slots.actions.is_empty();
        let drawer = matches!(kind, ModalSurfaceKind::Drawer(_));
        let text_height =
            title.height + description.map_or(0.0, |metrics| spacing.xs + metrics.height);
        let (icon_size, close_size, gap) = if drawer {
            (
                DRAWER_ICON_SIZE,
                ControlSize::Small.height_in(theme.metrics()),
                spacing.lg,
            )
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
                spacing.xl * 2.0 + action_height
            } else {
                0.0
            };
            return Self {
                header_height: spacing.xxl * 2.0 + row_height,
                footer_height,
                body_pad_top: spacing.md,
                body_pad_bottom: spacing.md,
                header_x: spacing.xxxl,
                body_x: spacing.xxxl,
                footer_x: spacing.xxxl,
                action_gap: spacing.md,
                row_top: spacing.xxl,
                row_height,
                text_height,
                icon,
                close,
                gap,
                footer_top: 0.0,
                action_top: spacing.xl,
                action_height,
                header_divider: 0.0,
                footer_divider: 0.0,
                inset_dividers: false,
                title_max_width: None,
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
                footer_top + action_height + recipe.footer.bottom
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
            action_height,
            header_divider,
            footer_divider,
            inset_dividers: recipe.divider_inset == nana_ui_core::DialogDividerInset::Content,
            title_max_width: recipe.title_max_width,
            drawer,
        }
    }

    /// How wide the title and its description may run: the header row less
    /// the icon and the close button with their gaps, and no wider than the
    /// recipe's title width over a card `surface` in its `scrim`.
    pub fn text_width(self, surface: crate::LayoutBox, scrim: crate::LayoutBox) -> f32 {
        let icon = self.icon.map_or(0.0, |size| size + self.gap);
        let close = self.close.map_or(0.0, |size| self.gap + size);
        let row = (surface.width - self.header_x * 2.0 - icon - close).max(0.0);
        let cap = self
            .title_max_width
            .and_then(|length| {
                length.resolve_with(Some(surface.width), Some((scrim.width, scrim.height)))
            })
            .filter(|width| width.is_finite());
        cap.map_or(row, |cap| row.min(cap.max(0.0)))
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
            (band_top + self.footer_top, self.action_height)
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
        (self.header_divider > 0.0).then(|| {
            let (x, width) = self.divider_span(surface, self.header_x);
            crate::LayoutBox {
                x,
                y: surface.y + self.header_height - self.header_divider,
                width,
                height: self.header_divider,
            }
        })
    }

    /// The hairline over the footer, when the recipe draws one.
    pub fn footer_divider_box(self, surface: crate::LayoutBox) -> Option<crate::LayoutBox> {
        (self.footer_divider > 0.0).then(|| {
            let (x, width) = self.divider_span(surface, self.footer_x);
            crate::LayoutBox {
                x,
                y: surface.y + surface.height - self.footer_height,
                width,
                height: self.footer_divider,
            }
        })
    }

    /// Where a hairline runs across the card: edge to edge, or inside the
    /// section's `inline` inset.
    fn divider_span(self, surface: crate::LayoutBox, inline: f32) -> (f32, f32) {
        if self.inset_dividers {
            (surface.x + inline, (surface.width - inline * 2.0).max(0.0))
        } else {
            (surface.x, surface.width)
        }
    }
}

/// The type a modal frame's title is set in: the recipe's for a dialog
/// card, the fixed section title for a drawer, which keeps its own chrome.
/// Shaping and paint both read it, so the measured title is the drawn one.
pub(crate) fn modal_title_type(
    kind: ModalSurfaceKind,
    recipe: &nana_ui_core::DialogRecipe,
    typography: nana_ui_core::TypographyTokens,
) -> nana_ui_core::RecipeText {
    match kind {
        ModalSurfaceKind::Drawer(_) => {
            nana_ui_core::RecipeText::new(typography.section, Some(typography.semibold))
        }
        ModalSurfaceKind::Dialog(_) | ModalSurfaceKind::Confirm(_) => recipe.title,
    }
}

pub(crate) fn drawer_width(viewport_width: f32) -> f32 {
    DRAWER_WIDTH.min(viewport_width * 0.92)
}

/// Where a modal surface sits in its scrim `bounds`. A dialog card is as
/// wide as its size asks and stands where the theme's [`DialogRecipe`] puts
/// it, at its `top` or centred; both give way to the scrim's margin, so the
/// card stays inside it.
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
            let top = match recipe.align {
                nana_ui_core::DialogAlign::Top => recipe.top_in(bounds.width, bounds.height),
                nana_ui_core::DialogAlign::Center => (bounds.height - height) / 2.0,
            }
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

    const MODAL_BODY_TEXT_SIZE: f32 = nana_ui_core::TypographyTokens::DEFAULT.body;

    /// The built-in dialog card's header and body insets.
    const HEADER: nana_ui_core::DialogInsets = nana_ui_core::DialogRecipe::DEFAULT.header;
    const BODY: nana_ui_core::DialogInsets = nana_ui_core::DialogRecipe::DEFAULT.body;
    use crate::{AppContext, Button, DocumentId, MountState};
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
            footer: None,
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
        let ModalFrameGeometry {
            scrim,
            surface,
            body: body_region,
            ..
        } = cx.world().modal_frame_geometry(drawer.stable_id()).unwrap();
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
        Arc::make_mut(&mut translated_style.layout)
            .transforms
            .transform = Some(nana_ui_core::PaintTransform {
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
            let ModalFrameGeometry { surface, .. } =
                cx.world().modal_frame_geometry(drawer.stable_id()).unwrap();
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

    /// A centred recipe stands the card midway down the scrim whatever its
    /// height, a card past the recipe's tallest included, and never past
    /// the scrim's margin; the built-in recipe keeps it at its top.
    #[test]
    fn a_centred_recipe_stands_the_card_midway_down_the_scrim() {
        use nana_ui_core::{DialogAlign, DialogRecipe};
        let scrim = crate::LayoutBox {
            x: 0.0,
            y: 40.0,
            width: 1200.0,
            height: 800.0,
        };
        let centred = DialogRecipe {
            align: DialogAlign::Center,
            ..DialogRecipe::DEFAULT
        };
        let confirm = ModalSurfaceKind::Confirm(DialogSize::Compact);
        for height in [120.0, 300.0] {
            let surface = modal_surface_bounds(scrim, confirm, Some(height), &centred);
            assert_eq!(surface.height, height);
            assert!(
                (surface.y - (scrim.y + (scrim.height - height) / 2.0)).abs() < 0.01,
                "{surface:?}"
            );
        }
        let tall = modal_surface_bounds(scrim, confirm, Some(2000.0), &centred);
        assert!((tall.height - 800.0 * 0.76).abs() < 0.01, "{tall:?}");
        assert!((tall.y - (scrim.y + 96.0)).abs() < 0.01, "{tall:?}");

        let cramped = crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 100.0,
        };
        let inside = modal_surface_bounds(cramped, confirm, Some(90.0), &centred);
        assert!(inside.y >= 16.0 - 0.01, "{inside:?}");
        assert!(inside.y + inside.height <= cramped.height - 16.0 + 0.01);

        let top = modal_surface_bounds(scrim, confirm, Some(120.0), &DialogRecipe::DEFAULT);
        assert_eq!(top.y, scrim.y + 90.0);
    }

    /// Laid out, a centred card carries its slots: the body stands where
    /// the card's body is, midway down the window.
    #[test]
    fn a_centred_card_carries_its_slots() {
        use nana_ui_core::{DialogAlign, DialogRecipe, LengthSpec};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(
            DialogRecipe {
                align: DialogAlign::Center,
                ..DialogRecipe::DEFAULT
            },
        ))
        .unwrap();
        let host = cx
            .create_component(document, crate::OverlayHost::new())
            .unwrap();
        let dialog = cx
            .create_detached_component(document, crate::Dialog::new("Export"))
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        let body = cx
            .create_detached_component(
                document,
                crate::Stack::column(0.0).height(LengthSpec::Px(200.0)),
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
        let (surface, body_region) = dialog_frame(&cx, dialog.stable_id());
        let above = surface.y;
        let below = 800.0 - (surface.y + surface.height);
        assert!((above - below).abs() < 0.01, "{surface:?}");
        let body_box = cx.world().canonical_layout_box(body.stable_id()).unwrap();
        assert_eq!(body_box.y, body_region.y, "the body slot is in the card");
        assert!(body_box.y + body_box.height <= surface.y + surface.height);
    }

    fn dialog_frame(cx: &AppContext, dialog: StableNodeId) -> (crate::LayoutBox, crate::LayoutBox) {
        let ModalFrameGeometry { surface, body, .. } =
            cx.world().modal_frame_geometry(dialog).unwrap();
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

    fn modal_geometry(cx: &AppContext, modal: StableNodeId) -> ModalFrameGeometry {
        cx.world().modal_frame_geometry(modal).unwrap()
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
        let ModalFrameGeometry {
            surface,
            body,
            title,
            ..
        } = modal_geometry(&cx, dialog.stable_id());
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
        let ModalFrameGeometry { title, .. } = modal_geometry(&cx, plain.stable_id());
        assert_eq!(title.color, Some(text));
        let ModalFrameGeometry { title, .. } = modal_geometry(&cx, confirm.stable_id());
        assert_eq!(title.color, Some(danger));
    }

    /// A title content slot takes the title text's place in the header
    /// row: after the icon and before the close button, as wide as the
    /// title may run, centred in the row and as tall as it lays out. The
    /// title stays the dialog's accessible name and is no longer drawn.
    /// Hiding the slot or taking it away gives the row back to the title,
    /// exactly as it was.
    #[test]
    fn a_title_content_slot_takes_the_titles_place_in_the_header_row() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dialog = cx
            .create_component(document, crate::Dialog::new("资源库"))
            .unwrap();
        let icon = cx
            .create_detached_component(document, crate::Stack::column(0.0))
            .unwrap()
            .stable_id();
        let close = cx
            .create_detached_component(
                document,
                crate::IconButton::new(nana_ui_core::Icon::Close, "Close"),
            )
            .unwrap()
            .stable_id();
        let text_only = ModalSlots {
            title_icon: Some(icon),
            close_action: Some(close),
            ..Default::default()
        };
        cx.set_modal_slots(dialog, text_only.clone()).unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let plain = modal_geometry(&cx, dialog.stable_id());

        let content = cx
            .create_detached_component(
                document,
                crate::Stack::row(0.0).height(LengthSpec::Px(40.0)),
            )
            .unwrap();
        cx.set_modal_slots(
            dialog,
            ModalSlots {
                title_content: Some(content.stable_id()),
                ..text_only.clone()
            },
        )
        .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        assert_eq!(
            cx.world().node(dialog.stable_id()).unwrap().children,
            vec![icon, content.stable_id(), close],
            "the row holds the icon, the content and the close button in order"
        );
        let ModalFrameGeometry {
            surface,
            body,
            title,
            ..
        } = modal_geometry(&cx, dialog.stable_id());
        assert!(title.content.is_empty(), "the title text is not drawn");
        assert_eq!(
            cx.world()
                .accessibility(dialog.stable_id())
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("资源库"),
            "the title stays the accessible name"
        );
        let recipe = nana_ui_core::DialogRecipe::DEFAULT;
        let icon_box = cx.world().canonical_layout_box(icon).unwrap();
        let close_box = cx.world().canonical_layout_box(close).unwrap();
        let content_box = cx
            .world()
            .canonical_layout_box(content.stable_id())
            .unwrap();
        assert_eq!(
            content_box.x,
            icon_box.x + recipe.icon_size + recipe.header_gap
        );
        assert!(
            (content_box.width
                - (surface.width
                    - MODAL_PAD_X * 2.0
                    - recipe.icon_size
                    - recipe.header_gap * 2.0
                    - recipe.close_size))
                .abs()
                < 0.01,
            "{content_box:?}"
        );
        assert_eq!(content_box.height, 40.0);
        let middle = content_box.y + content_box.height / 2.0;
        for square in [icon_box, close_box] {
            assert!(
                (square.y + square.height / 2.0 - middle).abs() < 0.01,
                "{square:?} is centred on {content_box:?}"
            );
        }
        assert_eq!(
            body.y,
            surface.y + HEADER.top + 40.0 + HEADER.bottom + BODY.top,
            "the row is as tall as the content"
        );

        cx.update_component(content, |stack, _| stack.layout_mut().hidden = true)
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        assert_eq!(
            modal_geometry(&cx, dialog.stable_id()),
            plain,
            "a hidden slot gives the row back to the title"
        );

        cx.update_component(content, |stack, _| stack.layout_mut().hidden = false)
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        let ModalFrameGeometry { title, .. } = modal_geometry(&cx, dialog.stable_id());
        assert!(title.content.is_empty(), "shown again, it stands in again");

        cx.set_modal_slots(dialog, text_only).unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);
        assert_eq!(
            cx.world().node(dialog.stable_id()).unwrap().children,
            vec![icon, close]
        );
        assert_eq!(
            modal_geometry(&cx, dialog.stable_id()),
            plain,
            "without the slot the header is the text title's again"
        );
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
        let ModalFrameGeometry {
            surface,
            body,
            title,
            ..
        } = modal_geometry(&cx, dialog.stable_id());
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
        let ModalFrameGeometry {
            surface,
            title,
            corner_radius,
            header_divider: Some((header_rule, header_color)),
            footer_divider: Some((footer_rule, footer_color)),
            ..
        } = modal_geometry(&cx, dialog.stable_id())
        else {
            panic!("modal geometry")
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

        let ModalFrameGeometry { surface, .. } = modal_geometry(&cx, confirm.stable_id());
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
        let ModalFrameGeometry {
            surface,
            corner_radius,
            header_divider,
            footer_divider,
            ..
        } = modal_geometry(&cx, dialog.stable_id());
        assert_eq!(corner_radius, cx.world().theme().metrics().radius_md);
        assert_eq!((header_divider, footer_divider), (None, None));
        let footer_box = cx.world().canonical_layout_box(footer.stable_id()).unwrap();
        assert_eq!(footer_box.height, MODAL_ACTION_HEIGHT);
        assert_eq!(
            surface.y + surface.height,
            footer_box.y + footer_box.height + nana_ui_core::DialogRecipe::DEFAULT.footer.bottom
        );
    }

    /// Fills the header band, and keeps the sections it was handed for the
    /// test to read back.
    struct BandPainter {
        seen: Arc<Mutex<Vec<Option<ModalSurfaceSections>>>>,
    }

    impl crate::Painter for BandPainter {
        fn paint(&self, cx: &mut crate::PaintContext<'_>) {
            let sections = cx.modal_sections();
            self.seen.lock().unwrap().push(sections);
            let Some(sections) = sections else {
                return;
            };
            let mut header = crate::PaintPath::new();
            header.rounded_rect(sections.header, [0.0; 4]);
            cx.fill_path(&header, nana_ui_core::SemanticColorRole::Accent);
        }

        fn paint_key(&self) -> u64 {
            1
        }
    }

    /// A dialog that paints its own card: the painter records in the card's
    /// coordinates, reads where the header, title, body and footer lie, and
    /// is not asked again while nothing it depends on moved. A dialog
    /// without one keeps the theme's card.
    #[test]
    fn a_surface_painter_draws_the_card_from_its_sections_and_records_once() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let confirm = cx
            .create_component(
                document,
                ConfirmDialog::new("删除场景", "场景内的模型会一并删除。").surface_painter(
                    BandPainter {
                        seen: Arc::clone(&seen),
                    },
                ),
            )
            .unwrap();
        cx.assemble_confirm_dialog(confirm).unwrap();
        let plain = cx
            .create_component(document, crate::Dialog::new("重命名"))
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);

        let ModalFrameGeometry {
            surface,
            body,
            title,
            surface_paint: Some(recording),
            ..
        } = modal_geometry(&cx, confirm.stable_id())
        else {
            panic!("modal geometry")
        };
        let sections = seen
            .lock()
            .unwrap()
            .last()
            .copied()
            .flatten()
            .expect("recorded as a card surface");
        assert_eq!(
            (sections.header.x, sections.header.y, sections.header.width),
            (0.0, 0.0, surface.width)
        );
        assert_eq!(
            (sections.title.x, sections.title.y),
            (title.bounds.x - surface.x, title.bounds.y - surface.y)
        );
        assert!(sections.title.y + sections.title.height <= sections.header.height + 0.01);
        assert_eq!(sections.body.y, body.y - surface.y);
        let footer = sections.footer.expect("a confirm has actions");
        assert!((footer.y + footer.height - surface.height).abs() < 0.01);
        assert!(matches!(
            recording.behind_children.as_slice(),
            [crate::PaintOp::FillPath { .. }]
        ));
        assert!(recording.over_children.is_empty());

        let asked = seen.lock().unwrap().len();
        let ModalFrameGeometry {
            surface_paint: Some(again),
            ..
        } = modal_geometry(&cx, confirm.stable_id())
        else {
            panic!("modal geometry")
        };
        assert!(Arc::ptr_eq(&recording, &again), "the recording is reused");
        assert_eq!(seen.lock().unwrap().len(), asked, "and not asked again");

        let ModalFrameGeometry { surface_paint, .. } = modal_geometry(&cx, plain.stable_id());
        assert!(surface_paint.is_none(), "the theme's card");
    }

    /// A title held to the tab wraps at the tab, but the message still runs
    /// across the whole body: the card is exactly as tall as without the cap,
    /// with no lines reserved for a message measured at the title's width.
    #[test]
    fn a_capped_title_leaves_the_message_its_body_width() {
        use nana_ui_core::{DialogRecipe, LengthSpec};
        let measure = |title_max_width: Option<LengthSpec>| {
            let mut cx = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let recipe = DialogRecipe {
                title_max_width,
                ..DialogRecipe::DEFAULT
            };
            cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(recipe))
                .unwrap();
            let confirm = cx
                .create_component(
                    document,
                    ConfirmDialog::new(
                        "开启兼容",
                        "开启后，插件可以连接本机，每个插件第一次连接都需要你允许；这段说明足够长，在正文宽度里也要折成好几行，在标题的宽度里会折成更多行。",
                    ),
                )
                .unwrap();
            cx.assemble_confirm_dialog(confirm).unwrap();
            shaped_layout(&mut cx, document, 800.0, 600.0);
            let ModalFrameGeometry {
                surface,
                body_text: Some(message),
                ..
            } = modal_geometry(&cx, confirm.stable_id())
            else {
                panic!("modal geometry")
            };
            (surface.height, message.bounds.height)
        };
        let free = measure(None);
        let capped = measure(Some(LengthSpec::Percent(30.0)));
        assert_eq!(capped, free);
    }

    /// A card whose header stands on a tab: the recipe sets the title's
    /// type, keeps a danger title in the text colour, holds the title inside
    /// the tab's share of the card and starts the rule where the heading
    /// does. The header band a surface painter reads ends where the rule
    /// starts. A drawer under the same theme keeps its own title.
    #[test]
    fn a_tab_headed_recipe_sets_the_title_and_keeps_it_and_the_rule_inside() {
        use nana_ui_core::{
            DialogDividerInset, DialogRecipe, LengthSpec, RecipeText, SemanticColorRole, StatusTone,
        };
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let recipe = DialogRecipe {
            title: RecipeText::new(15.0, Some(700)),
            danger_title: false,
            title_max_width: Some(LengthSpec::Percent(40.0)),
            header_divider: Some(SemanticColorRole::BorderSoft),
            divider_inset: DialogDividerInset::Content,
            ..DialogRecipe::DEFAULT
        };
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(recipe))
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let confirm = cx
            .create_component(
                document,
                ConfirmDialog::new(
                    "删除这个场景以及它里面的全部模型和挂件",
                    "场景内的模型、挂件与画布设置会一并删除。",
                )
                .danger(true)
                .surface_painter(BandPainter {
                    seen: Arc::clone(&seen),
                }),
            )
            .unwrap();
        cx.assemble_confirm_dialog(confirm).unwrap();
        let drawer = cx
            .create_component(document, Drawer::new("检查器"))
            .unwrap();
        shaped_layout(&mut cx, document, 800.0, 600.0);

        let theme = cx.world().theme();
        let danger = theme
            .style_model()
            .color(theme.recipes().status().role(StatusTone::Danger))
            .as_rgba_array();
        let ModalFrameGeometry {
            surface,
            title,
            header_divider: Some((rule, _)),
            ..
        } = modal_geometry(&cx, confirm.stable_id())
        else {
            panic!("modal geometry")
        };
        assert_eq!((title.font_size, title.font_weight), (15.0, Some(700)));
        assert_ne!(title.color, Some(danger), "danger shows elsewhere");
        assert!(
            title.bounds.width <= surface.width * 0.4 + 0.01,
            "{title:?} runs past the tab of {surface:?}"
        );
        let inline = recipe.header.inline;
        assert_eq!(
            (rule.x, rule.width),
            (surface.x + inline, surface.width - inline * 2.0)
        );
        let sections = seen
            .lock()
            .unwrap()
            .last()
            .copied()
            .flatten()
            .expect("recorded as a card surface");
        assert_eq!(sections.header.height, rule.y - surface.y);

        let ModalFrameGeometry { title, .. } = modal_geometry(&cx, drawer.stable_id());
        assert_eq!(
            (title.font_size, title.font_weight),
            (
                nana_ui_core::type_scale::SECTION,
                Some(nana_ui_core::type_scale::SEMIBOLD)
            )
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
            let ModalFrameGeometry {
                scrim_color,
                scrim_blur,
                ..
            } = modal_geometry(&cx, modal);
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
            let ModalFrameGeometry {
                scrim_color,
                scrim_blur,
                ..
            } = modal_geometry(&cx, modal);
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
        let ModalFrameGeometry {
            surface,
            scrim_opacity,
            ..
        } = modal_geometry(&cx, id);
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
        let ModalFrameGeometry { scrim_opacity, .. } = modal_geometry(&cx, id);
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

    /// A shorter fade must keep its zero endpoint while movement continues;
    /// reopening in that interval reverses from zero without the old exit
    /// completion releasing the host. A completed exit can also be parked and
    /// remounted without inheriting the logical opacity of one.
    #[test]
    fn a_dialog_holds_finished_exit_tracks_until_reopening() {
        use crate::{AnimatableProperty, MotionValue};
        use std::time::Duration;
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        cx.set_theme_definition(
            &nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(moving_recipe()),
        )
        .unwrap();
        let (host, dialog, _) = hosted_dialog(&mut cx, document);
        let id = dialog.stable_id();
        cx.activate_overlay(host, dialog).unwrap();
        cx.advance_animations(Duration::from_millis(200));
        cx.dismiss_overlay(host).unwrap();
        cx.advance_animations(Duration::from_millis(320));
        assert!(cx.world().surface_closing(id));
        assert_eq!(
            presented(&cx, id, AnimatableProperty::Opacity),
            Some(MotionValue::Scalar(0.0))
        );
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(id)
        );
        assert!(cx.activate_overlay(host, dialog).unwrap());
        assert_eq!(
            presented(&cx, id, AnimatableProperty::Opacity),
            Some(MotionValue::Scalar(0.0))
        );
        cx.advance_animations(Duration::from_millis(400));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(id)
        );
        cx.advance_animations(Duration::from_millis(520));
        cx.dismiss_overlay(host).unwrap();
        cx.advance_animations(Duration::from_millis(720));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        assert_eq!(
            cx.next_animation_deadline(),
            None,
            "holds do not keep ticking"
        );
        let mut mutations = crate::MutationQueue::new();
        mutations.park_subtree(id);
        cx.commit_mutations(mutations).unwrap();
        cx.append_child(host, dialog).unwrap();
        cx.activate_overlay(host, dialog).unwrap();
        assert_eq!(
            presented(&cx, id, AnimatableProperty::Opacity),
            Some(MotionValue::Scalar(0.0))
        );
        assert_eq!(cx.world().node_style(id).unwrap().layout.opacity, None);
        cx.advance_animations(Duration::from_millis(780));
        let Some(MotionValue::Scalar(alpha)) = presented(&cx, id, AnimatableProperty::Opacity)
        else {
            panic!("reopening fades in")
        };
        assert!((alpha - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_dialog_holds_its_exit_transform_while_the_scrim_finishes() {
        use crate::{AnimatableProperty, MotionValue};
        use std::time::Duration;
        let mut cx = AppContext::new();
        let mut recipe = moving_recipe();
        recipe.motion.card_move = nana_ui_core::DialogTransition::new(80, crate::Easing::Linear);
        cx.set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_dialog(recipe))
            .unwrap();
        let (host, dialog, _) = hosted_dialog(&mut cx, DocumentId::new(1).unwrap());
        let id = dialog.stable_id();
        cx.activate_overlay(host, dialog).unwrap();
        cx.advance_animations(Duration::from_millis(200));
        cx.dismiss_overlay(host).unwrap();
        cx.advance_animations(Duration::from_millis(280));
        let Some(MotionValue::Transform(transform)) =
            presented(&cx, id, AnimatableProperty::Transform)
        else {
            panic!("finished movement holds its endpoint")
        };
        assert_eq!((transform.a, transform.f), (0.98, -8.0));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(id)
        );
        cx.advance_animations(Duration::from_millis(320));
        assert_eq!(
            presented(&cx, id, AnimatableProperty::Opacity),
            Some(MotionValue::Scalar(0.0))
        );
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            Some(id)
        );
        cx.advance_animations(Duration::from_millis(360));
        assert_eq!(
            cx.world().overlay_host(host.stable_id()).unwrap().active,
            None
        );
        assert_eq!(cx.next_animation_deadline(), None);
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

        let ModalFrameGeometry {
            surface,
            title,
            description: Some(description),
            body,
            ..
        } = cx.world().modal_frame_geometry(dialog.stable_id()).unwrap()
        else {
            panic!("modal geometry")
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
        let ModalFrameGeometry {
            surface,
            title,
            description,
            body_text: Some(message),
            border,
            ..
        } = cx
            .world()
            .modal_frame_geometry(confirm.stable_id())
            .unwrap()
        else {
            panic!("modal geometry")
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
                    footer: None,
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
            let ModalFrameGeometry {
                title,
                body_text: Some(message),
                ..
            } = cx
                .world()
                .modal_frame_geometry(confirm.stable_id())
                .unwrap()
            else {
                panic!("modal geometry")
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

    /// A confirmation's footer stands at the start of its action row: on the
    /// actions' line and as tall as they are, from the footer's inset to the
    /// leftmost action less the gap. It adds no height and moves no action,
    /// comes before the actions in the dialog's order, and the dialog still
    /// opens on its first action.
    #[test]
    fn confirm_footer_stands_at_the_start_of_the_action_row() {
        struct Laid {
            surface: crate::LayoutBox,
            footer: Option<crate::LayoutBox>,
            cancel: crate::LayoutBox,
            accept: crate::LayoutBox,
        }
        fn lay_out(with_footer: bool) -> Laid {
            let mut cx = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let host = cx
                .create_component(document, crate::OverlayHost::new())
                .unwrap();
            let confirm = cx
                .create_component(
                    document,
                    ConfirmDialog::new("Turn it on", "Plugins may connect once it is on."),
                )
                .unwrap();
            let footer = with_footer.then(|| {
                cx.create_detached_component(document, crate::Switch::new("Don't ask again", false))
                    .unwrap()
            });
            let cancel = cx
                .create_detached_component(document, Button::new("Cancel"))
                .unwrap();
            let accept = cx
                .create_detached_component(document, Button::new("Turn on"))
                .unwrap();
            cx.set_confirm_slots(
                confirm,
                ConfirmSlots {
                    title_icon: None,
                    body: None,
                    footer: footer.map(|footer| footer.stable_id()),
                    close_action: None,
                    cancel: cancel.stable_id(),
                    secondary: None,
                    confirm: accept.stable_id(),
                },
            )
            .unwrap();
            let children = cx.world().node(confirm.stable_id()).unwrap().children;
            let actions = vec![cancel.stable_id(), accept.stable_id()];
            match footer {
                Some(footer) => assert_eq!(
                    children,
                    [vec![footer.stable_id()], actions].concat(),
                    "the footer comes before the actions"
                ),
                None => assert_eq!(children, actions),
            }
            cx.append_child(host, confirm).unwrap();
            assert!(cx.activate_overlay(host, confirm).unwrap());
            assert_eq!(
                cx.world().focused(document),
                Some(cancel.stable_id()),
                "it opens on its first action"
            );
            let work = cx.take_system_work();
            cx.resolve_styles(&work.style).unwrap();
            let mut shaper = WrappingShaper;
            cx.shape_text(&work.text, &mut shaper).unwrap();
            cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
                .unwrap();
            let world = cx.world();
            let ModalFrameGeometry { surface, .. } =
                world.modal_frame_geometry(confirm.stable_id()).unwrap();
            Laid {
                surface,
                footer: footer.map(|footer| world.layout_box(footer.stable_id()).unwrap()),
                cancel: world.layout_box(cancel.stable_id()).unwrap(),
                accept: world.layout_box(accept.stable_id()).unwrap(),
            }
        }

        let bare = lay_out(false);
        let with = lay_out(true);
        let footer = with.footer.expect("a footer box");
        let recipe = nana_ui_core::DialogRecipe::DEFAULT;
        assert_eq!(with.surface, bare.surface, "the footer adds no height");
        assert_eq!((with.cancel, with.accept), (bare.cancel, bare.accept));
        assert!((footer.x - (with.surface.x + recipe.footer.inline)).abs() < 0.01);
        assert!((footer.y - with.cancel.y).abs() < 0.01);
        assert!((footer.height - with.cancel.height).abs() < 0.01);
        assert!(
            (footer.x + footer.width - (with.cancel.x - recipe.action_gap)).abs() < 0.01,
            "{footer:?} beside {:?}",
            with.cancel
        );
    }

    /// The keyboard contract of a modal dialog, through the real key router:
    /// Enter activates the action focus is on (the first action when it
    /// opens, or the `initial_focus` target), Escape closes it.
    #[test]
    fn enter_activates_the_focused_default_action_and_escape_closes_the_dialog() {
        use nana_ui_input::{KeyInput, KeyState};
        fn press(cx: &mut AppContext, document: DocumentId, key: &'static str) {
            let mut input = crate::HeadlessInput::bind(cx, document);
            input
                .press(
                    cx,
                    KeyInput::named(key, key, KeyState::Pressed, Default::default()),
                    None,
                    None,
                )
                .unwrap();
        }
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
        let commit = cx
            .create_detached_component(document, Button::new("Delete"))
            .unwrap();
        cx.set_confirm_slots(
            confirm,
            ConfirmSlots {
                title_icon: None,
                body: None,
                footer: None,
                close_action: None,
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
        let closes = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&closes);
        cx.on(
            confirm,
            move |_dialog, request: &crate::DialogCloseRequested, _| {
                heard.lock().unwrap().push(request.trigger)
            },
        )
        .unwrap();
        cx.append_child(host, confirm).unwrap();
        let open = |cx: &mut AppContext| {
            assert!(cx.activate_overlay(host, confirm).unwrap());
            cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
                .unwrap();
        };
        // Open as far as the keyboard and the application are concerned; the
        // host keeps the closing card until its exit motion ends.
        let active = |cx: &AppContext| {
            cx.active_runtime_overlay(document).is_some()
                && cx.read(confirm, |dialog| dialog.open).unwrap()
        };

        // Opened, focus is on the first action; Enter activates it.
        open(&mut cx);
        assert_eq!(cx.world().focused(document), Some(cancel.stable_id()));
        press(&mut cx, document, "Enter");
        assert_eq!(*intents.lock().unwrap(), vec![ConfirmIntent::Cancel]);

        // Escape asks first, then the default policy closes it.
        assert!(active(&cx));
        press(&mut cx, document, "Escape");
        assert_eq!(
            *closes.lock().unwrap(),
            vec![nana_ui_core::DialogCloseTrigger::Escape]
        );
        assert!(!active(&cx));

        // With the confirming action as the default, Enter confirms.
        cx.update_component(confirm, |dialog, _| {
            dialog.set_initial_focus(ModalInitialFocus::Target(commit.stable_id()))
        })
        .unwrap();
        intents.lock().unwrap().clear();
        open(&mut cx);
        assert_eq!(cx.world().focused(document), Some(commit.stable_id()));
        press(&mut cx, document, "Enter");
        assert_eq!(
            *intents.lock().unwrap(),
            vec![ConfirmIntent::Confirm { danger: false }]
        );
        press(&mut cx, document, "Escape");
        assert!(!active(&cx));
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
                footer: None,
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
        assert_eq!(
            cx.world()
                .widget_visual_as::<crate::ButtonVisual>(commit.stable_id())
                .unwrap()
                .effective_in(commit.stable_id(), cx.world()),
            (nana_ui_core::ButtonKind::Danger, true),
        );
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
        assert_eq!(
            cx.world()
                .widget_visual_as::<crate::ButtonVisual>(commit.stable_id())
                .unwrap()
                .effective_in(commit.stable_id(), cx.world()),
            (nana_ui_core::ButtonKind::Primary, false),
        );
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
