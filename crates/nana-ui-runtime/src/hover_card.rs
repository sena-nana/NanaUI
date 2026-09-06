//! Hover-triggered anchored card. The trigger stays in flow; the card content
//! hangs off it while the pointer rests on the trigger or on the card itself.
//!
//! Unlike [`crate::Popover`] the surface opens from pointer hover, not
//! activation, and its content stays interactive: moving the pointer onto the
//! card keeps it open, leaving both trigger and card closes it after a short
//! grace delay. The surface carries no padding of its own — the content child
//! owns its padding so the hover-safe area covers the whole card.

use std::sync::Arc;

use nana_ui_core::{
    ContentFit, Icon, LengthSpec, OverflowSpec, PopoverAlignment, PopoverPlacement,
    SemanticColorRole,
};

use crate::gpu_slots::HOST_TEXTURE_RENDERER;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    MenuSurfaceKind, MutationQueue, NodeKind, NodeStyle, StandardVisual, StableNodeId,
    TriggeredMenuOverlay, UiWorld,
};
use crate::popover::{trigger_button_style, trigger_icon_button_style};

const HOVER_CARD_WIDTH: f32 = 240.0;
const HOVER_CARD_GAP: f32 = 6.0;
/// Grace period after the pointer leaves both trigger and card.
pub const DEFAULT_CLOSE_DELAY_MS: u64 = 120;

/// Hover-opened anchored card (`nana.hover-card`). The trigger renders in
/// flow; card content mounts as children and projects onto the anchored
/// surface while open. `open` is framework-driven: hover lifecycle owns it.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverCard {
    /// Text trigger label, or the accessible name when the trigger is a
    /// glyph or an avatar.
    pub trigger: Arc<str>,
    pub trigger_icon: Option<Icon>,
    /// Host-texture resource for an avatar trigger. The circular chrome comes
    /// from the style; an empty resource renders the neutral placeholder.
    pub trigger_image: Option<Arc<str>>,
    pub trigger_size: f32,
    pub open: bool,
    pub placement: PopoverPlacement,
    pub alignment: PopoverAlignment,
    pub gap: f32,
    pub width: f32,
    pub open_delay_ms: u64,
    pub close_delay_ms: u64,
    pub close_on_escape: bool,
}

impl HoverCard {
    pub fn new() -> Self {
        Self {
            trigger: Arc::from(""),
            trigger_icon: None,
            trigger_image: None,
            trigger_size: crate::avatar::DEFAULT_SIZE,
            open: false,
            placement: PopoverPlacement::Right,
            alignment: PopoverAlignment::Center,
            gap: HOVER_CARD_GAP,
            width: HOVER_CARD_WIDTH,
            open_delay_ms: 300,
            close_delay_ms: DEFAULT_CLOSE_DELAY_MS,
            close_on_escape: true,
        }
    }

    /// Text trigger with its label.
    pub fn trigger(mut self, trigger: impl Into<Arc<str>>) -> Self {
        self.trigger = trigger.into();
        self
    }

    /// Glyph trigger; the label stays the accessible name only.
    pub fn trigger_icon(mut self, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        self.trigger = label.into();
        self.trigger_icon = Some(icon);
        self
    }

    /// Avatar trigger backed by a host-texture resource; the label is the
    /// accessible name.
    pub fn trigger_image(mut self, resource: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        self.trigger = label.into();
        self.trigger_image = Some(resource.into());
        self
    }

    pub fn trigger_size(mut self, size: f32) -> Self {
        self.trigger_size = sanitize_size(size);
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
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(crate::popover::MENU_MIN_WIDTH);
        self
    }

    pub fn open_delay(mut self, ms: u64) -> Self {
        self.open_delay_ms = ms;
        self
    }

    pub fn close_delay(mut self, ms: u64) -> Self {
        self.close_delay_ms = ms;
        self
    }

    pub fn close_on_escape(mut self, enabled: bool) -> Self {
        self.close_on_escape = enabled;
        self
    }

    /// Image trigger resource without the placeholder check, mirroring
    /// [`Avatar::custom_render`].
    fn custom_render(&self) -> Option<CustomRenderNode> {
        let resource = self.trigger_image.as_deref()?.trim();
        if resource.is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(HOST_TEXTURE_RENDERER, Arc::from(resource), 0)
                .with_fit(ContentFit::Cover),
        )
    }

    fn effective_style(&self) -> NodeStyle {
        if self.trigger_image.is_some() {
            return self.image_trigger_style();
        }
        if self.trigger_icon.is_some() {
            return trigger_icon_button_style();
        }
        trigger_button_style()
    }

    /// Avatar chrome: circular clip, fixed box, neutral placeholder while the
    /// host texture is absent.
    fn image_trigger_style(&self) -> NodeStyle {
        let size = sanitize_size(self.trigger_size);
        let mut style = NodeStyle {
            background: Some(SemanticColorRole::Subtle),
            ..NodeStyle::default()
        };
        if self.custom_render().is_some() {
            style.background = None;
        }
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(size));
        layout.height = Some(LengthSpec::Px(size));
        layout.min_width = Some(LengthSpec::Px(size));
        layout.min_height = Some(LengthSpec::Px(size));
        layout.max_width = Some(LengthSpec::Px(size));
        layout.max_height = Some(LengthSpec::Px(size));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.border_width = Some(0.0);
        layout.border_radius = Some(size * 0.5);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        style
    }
}

impl Default for HoverCard {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for HoverCard {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "hover-card".into(),
        }
    }

    /// The card content mounts and detaches under the anchored surface, so
    /// the projection must re-run with it to keep the origin math current.
    fn wants_child_reproject() -> bool {
        true
    }

    /// The framework flips `open` through the hover lifecycle; projection
    /// only mirrors it into the retained surface.
    fn wants_hover_tracking() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let open = world.project_menu_presence(id, self.open, mutations);
        let trigger_image = self
            .trigger_image
            .as_ref()
            .filter(|resource| !resource.trim().is_empty())
            .cloned();
        let trigger = (self.trigger_icon.is_none() && trigger_image.is_none())
            .then(|| Arc::clone(&self.trigger))
            .filter(|label| !label.is_empty());
        let visual = StandardVisual::MenuSurface {
            kind: MenuSurfaceKind::HoverCard,
            open,
            trigger: trigger.clone(),
            trigger_icon: self.trigger_icon,
            trigger_image,
            gap: self.gap,
            overlay: Some(TriggeredMenuOverlay {
                placement: self.placement,
                alignment: self.alignment,
                width: self.width.max(crate::popover::MENU_MIN_WIDTH),
                padding: 0.0,
                gap: self.gap,
            }),
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        // Glyph and avatar triggers carry no text; their label lives in the
        // accessibility state only.
        let text = if self.trigger_icon.is_some() || self.trigger_image.is_some() {
            ""
        } else {
            self.trigger.as_ref()
        };
        if world.text(id) != Some(text) {
            mutations.set_text(
                id,
                crate::TextContent {
                    value: text.to_string(),
                },
            );
        }
        let custom = self.custom_render();
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        let label = (!self.trigger.is_empty()).then(|| Arc::clone(&self.trigger));
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label,
                ..AccessibilityState::default()
            },
        );
    }
}

fn sanitize_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        crate::avatar::DEFAULT_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn tick(context: &mut AppContext, at_ms: u64) {
        context.advance_animations(std::time::Duration::from_millis(at_ms));
    }

    fn hover_at(context: &mut AppContext, document: DocumentId, target: Option<StableNodeId>, at_ms: u64) {
        context
            .set_pointer_hover_at(document, 1, target, std::time::Duration::from_millis(at_ms))
            .unwrap();
    }

    fn card_with_button() -> (AppContext, crate::Entity<HoverCard>, crate::Entity<crate::Button>) {
        let mut context = AppContext::new();
        let card = context
            .create_component(
                document(),
                HoverCard::new().trigger("账户").open_delay(0).close_delay(120),
            )
            .unwrap();
        let button = context
            .create_component(document(), crate::Button::new("退出登录"))
            .unwrap();
        context.append_child(card, button).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        (context, card, button)
    }

    fn relayout(context: &mut AppContext) {
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
    }

    /// Hovering the trigger opens the anchored surface after the configured
    /// delay, and the card content projects as a viewport-fixed overlay.
    #[test]
    fn hovering_the_trigger_opens_the_card_after_the_delay() {
        let (mut context, card, button) = card_with_button();
        context
            .update_component(card, |card, _| card.open_delay_ms = 100)
            .unwrap();
        let card_id = card.stable_id();
        hover_at(&mut context, document(), Some(card_id), 0);
        tick(&mut context, 40);
        assert!(!context.read(card, |card| card.open).unwrap());
        tick(&mut context, 140);
        assert!(context.read(card, |card| card.open).unwrap());
        relayout(&mut context);
        // The open card keeps its trigger in flow; the button floats free.
        let trigger_box = context.world().layout_box(card_id).unwrap();
        let button_box = context.world().layout_box(button.stable_id()).unwrap();
        let style = context.world().layout_style(button.stable_id()).unwrap();
        assert_eq!(style.position, nana_ui_core::PositionSpec::Fixed);
        assert!(
            button_box.x >= trigger_box.x + trigger_box.width,
            "card content hangs beside the trigger: trigger={trigger_box:?} button={button_box:?}"
        );
    }

    /// Moving the pointer onto the open card keeps it open past the close
    /// grace; only leaving both trigger and card closes it.
    #[test]
    fn moving_onto_the_card_keeps_it_open() {
        let (mut context, card, button) = card_with_button();
        let card_id = card.stable_id();
        let button_id = button.stable_id();
        hover_at(&mut context, document(), Some(card_id), 0);
        tick(&mut context, 400);
        assert!(context.read(card, |card| card.open).unwrap());
        hover_at(&mut context, document(), Some(button_id), 450);
        tick(&mut context, 800);
        assert!(
            context.read(card, |card| card.open).unwrap(),
            "pointer on the card content must not close it"
        );
        hover_at(&mut context, document(), None, 850);
        tick(&mut context, 1000);
        assert!(!context.read(card, |card| card.open).unwrap());
    }

    /// A scheduled open is cancelled when the pointer leaves before the
    /// delay elapses.
    #[test]
    fn leaving_before_the_delay_cancels_the_open() {
        let (mut context, card, _) = card_with_button();
        context
            .update_component(card, |card, _| card.open_delay_ms = 100)
            .unwrap();
        hover_at(&mut context, document(), Some(card.stable_id()), 0);
        hover_at(&mut context, document(), None, 40);
        tick(&mut context, 400);
        assert!(!context.read(card, |card| card.open).unwrap());
        assert_eq!(context.next_animation_deadline(), None);
    }

    /// Escape closes an open hover card that allows it.
    #[test]
    fn escape_closes_the_open_card() {
        let (mut context, card, _) = card_with_button();
        hover_at(&mut context, document(), Some(card.stable_id()), 0);
        tick(&mut context, 400);
        assert!(context.read(card, |card| card.open).unwrap());
        assert!(context.dismiss_popovers_on_escape().unwrap());
        assert!(!context.read(card, |card| card.open).unwrap());
    }

    /// The avatar trigger renders through the host-texture slot and keeps a
    /// neutral placeholder chrome while the texture is absent.
    #[test]
    fn avatar_trigger_carries_host_texture_and_placeholder() {
        let mut context = AppContext::new();
        let card = context
            .create_component(
                document(),
                HoverCard::new().trigger_image("user.avatar", "账户"),
            )
            .unwrap();
        let id = card.stable_id();
        let custom = context.world().custom_render(id).unwrap();
        assert_eq!(custom.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert!(context.world().text(id).is_none_or(|text| text.is_empty()));
        let style = context.world().node_style(id).unwrap();
        assert!(style.background.is_none(), "loaded avatars paint no chrome");
        assert_eq!(
            context.read(card, |card| card.trigger.as_ref().to_owned()).unwrap(),
            "账户"
        );

        let placeholder = context
            .create_component(document(), HoverCard::new().trigger_image("", "账户"))
            .unwrap();
        assert!(context.world().custom_render(placeholder.stable_id()).is_none());
        let placeholder_style = context
            .world()
            .node_style(placeholder.stable_id())
            .unwrap();
        assert_eq!(
            placeholder_style.background,
            Some(SemanticColorRole::Subtle)
        );
    }
}
