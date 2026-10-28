//! Compact selectable token (`nana.chip`). Visual language is a pill `Button`;
//! dismiss is a trailing close control assembled by [`AppContext::assemble_chip`].
//! Body emits [`crate::Activate`]; close emits [`ChipDismissed`]. Disabled emits neither.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ButtonKind, ControlHeight, ControlPadding, ControlSize, FlexDirection, Icon,
    LengthSpec, PositionSpec, SemanticColorRole, SpacingStep, ThemeExpr, ThemeToken,
};

use crate::view_components::{Activate, IconButton, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual,
    TextContent, UiWorld,
};

/// Request to remove a dismissible chip. Closing stays host-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipDismissed;

/// Compact selectable token (`nana.chip`).
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub label: Arc<str>,
    /// Canonical styled or mixed content. `None` uses the plain label.
    pub content: Option<Arc<crate::InlineContent>>,
    pub selected: bool,
    pub disabled: bool,
    pub dismissible: bool,
    /// The remove control's accessible name; `None` says the framework's
    /// (`chip.remove`).
    pub close_label: Option<Arc<str>>,
    pub size: ControlSize,
    pub style: NodeStyle,
    pub(crate) close: Option<StableNodeId>,
}

impl Chip {
    /// Text, spans and retained children share the label's content box.
    /// A plain value returns to the allocation-light string representation.
    pub fn content(mut self, content: impl Into<crate::InlineContent>) -> Self {
        let content = content.into().normalized();
        self.label = Arc::from(content.text());
        self.content = if matches!(content, crate::InlineContent::PlainText(_)) {
            None
        } else {
            Some(Arc::new(content))
        };
        self
    }
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            content: None,
            selected: false,
            disabled: false,
            dismissible: false,
            close_label: None,
            size: ControlSize::Small,
            style: NodeStyle::default(),
            close: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    pub fn close_label(mut self, close_label: impl Into<Arc<str>>) -> Self {
        self.close_label = Some(close_label.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn kind(&self) -> ButtonKind {
        if self.selected {
            ButtonKind::Selected
        } else {
            ButtonKind::Subtle
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(if self.selected {
            SemanticColorRole::Selected
        } else {
            SemanticColorRole::Subtle
        });
        style.border = (!self.selected).then_some(SemanticColorRole::BorderSoft);
        style.interaction.hovered.background = Some(if self.selected {
            SemanticColorRole::SelectedHover
        } else {
            SemanticColorRole::Hover
        });
        style.interaction.pressed.background = Some(if self.selected {
            SemanticColorRole::SelectedPressed
        } else {
            SemanticColorRole::Active
        });
        style.interaction.disabled.foreground = Some(SemanticColorRole::Faint);
        style.interaction.disabled.background = Some(SemanticColorRole::Subtle);
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Row);
            layout.align_items = AlignSpec::Center;
            layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
            if self.dismissible {
                // The label is the button's own measured text, not a flex child.
                // Reserve the trailing close box in the same content authority
                // that both intrinsic measurement and button paint consume.
                layout.position = PositionSpec::Relative;
                layout.theme.padding_right = Some(
                    ThemeExpr::token(ThemeToken::ControlPadding(ControlPadding::Compact))
                        .plus(ThemeToken::Control(ControlSize::Small).into())
                        .plus(ThemeToken::Space(SpacingStep::Xs).into()),
                );
            }
            layout.border_width = Some(nana_ui_core::HAIRLINE);
            layout.theme.border_width =
                Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
            layout.border_radius = Some(999.0);
            layout.font_size = Some(self.size.text_size());
            layout.theme.control_typography(self.size);
            layout.font_weight = Some(nana_ui_core::type_scale::MEDIUM);
            crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Medium);
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
            // Declared, not solved: only a strip that solves its overflow
            // reads it (Issue #212).
            layout
                .adaptation
                .get_or_insert(nana_ui_core::dynamic_layout::AdaptationProfile::control());
        }
        style.control_padding_x = Some(ControlPadding::Compact);
        style.control_height = Some(ControlHeight::Min(ControlSize::Small));
        style
    }
}

impl ComponentView for Chip {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_chip),
        assembler: Some(crate::AppContext::assemble_chip),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "chip".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let label = self
            .content
            .as_deref()
            .map_or(&self.label, crate::InlineContent::shared_text);
        if !crate::world::inline_content::project_control_content(
            id,
            world,
            &self.label,
            self.content.as_deref(),
            mutations,
        ) && world.text(id) != Some(label.as_ref())
        {
            mutations.set_text(
                id,
                TextContent {
                    value: Arc::clone(label).into(),
                },
            );
        }
        let visual = StandardVisual::Button {
            icon: None,
            trailing_icon: None,
            icon_size: self.size.icon_size(),
            icon_gap: nana_ui_core::space::SM,
            label: Arc::clone(label),
            kind: self.kind(),
            size: self.size,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
            content_align: crate::TextHorizontalAlignment::Center,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: self.content.is_none().then(|| Arc::clone(label)),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    /// Idempotently build the trailing close control when [`Chip::dismissible`].
    pub fn assemble_chip(&mut self, chip: Entity<Chip>) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(chip.stable_id())
            .ok_or(FrameworkError::MissingView(chip.stable_id()))?
            .document;
        let snapshot = self.read(chip, Clone::clone)?;
        if !snapshot.dismissible {
            if let Some(close) = snapshot.close.filter(|id| self.world().contains(*id)) {
                self.update_component(Entity::<IconButton>::from_stable_id(close), |button, _| {
                    Arc::make_mut(&mut button.style.layout).hidden = true;
                })?;
            }
            return Ok(false);
        }

        let close_label = snapshot
            .close_label
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.world().framework_strings().chip_remove));
        let created = snapshot.close.is_none();
        let close = match snapshot.close.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<IconButton>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                IconButton::new(Icon::Close, Arc::clone(&close_label))
                    .size(ControlSize::Small)
                    .kind(ButtonKind::Text)
                    .disabled(snapshot.disabled),
            )?,
        };
        if created {
            self.observe(close, chip, |chip, _: &Activate, cx| {
                if !chip.disabled {
                    cx.emit(ChipDismissed);
                }
            })?;
        }
        self.update_component(close, |button, _| {
            button.disabled = snapshot.disabled;
            button.label = close_label;
            button.style.square = None;
            let layout = Arc::make_mut(&mut button.style.layout);
            layout.hidden = false;
            layout.position = PositionSpec::Absolute;
            layout.theme.width = Some(ThemeToken::Control(ControlSize::Small).into());
            layout.height = Some(LengthSpec::Percent(100.0));
            // The chip is the containing block: the insets are measured from
            // its padding box, so the box sits in the reserved gutter, the
            // chip's own end padding in from its edge.
            layout.theme.offset_right = Some(ThemeExpr::token(ThemeToken::ControlPadding(
                ControlPadding::Compact,
            )));
        })?;
        self.update_component(chip, |chip, _| {
            chip.close = Some(close.stable_id());
        })?;
        self.append_child(chip, close)?;
        Ok(created)
    }

    /// If `id` is a Chip's assembled close control, return that Chip.
    pub fn chip_dismiss_target(&self, id: StableNodeId) -> Option<Entity<Chip>> {
        let parent = self.world().parent_id(id)?;
        let chip = self.view_entity::<Chip>(parent)?;
        let close = self.read(chip, |chip| chip.close).ok().flatten()?;
        (close == id).then_some(chip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn making_a_chip_dismissible_grows_its_close_button_without_a_second_call() {
        let mut context = AppContext::new();
        let chip = context
            .create_component(document(), Chip::new("Beta"))
            .unwrap();
        assert!(context.read(chip, |chip| chip.close).unwrap().is_none());

        // No explicit `assemble_chip`: the write itself reconciles the child.
        context
            .update_component(chip, |chip, _| chip.dismissible = true)
            .unwrap();
        let close = context
            .read(chip, |chip| chip.close)
            .unwrap()
            .expect("close button assembled from the prop change");
        assert!(context.world().contains(close));
    }

    #[test]
    fn pill_chrome_and_selected_kind() {
        let mut context = AppContext::new();
        let chip = context
            .create_component(document(), Chip::new("Beta").selected(true))
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 80.0))
            .unwrap();
        let visual = context.world().standard_visual(chip.stable_id()).unwrap();
        assert_eq!(
            visual,
            StandardVisual::Button {
                icon: None,
                trailing_icon: None,
                icon_size: ControlSize::Small.icon_size(),
                icon_gap: 6.0,
                label: Arc::from("Beta"),
                kind: ButtonKind::Selected,
                size: ControlSize::Small,
                loading: false,
                loading_phase: 0.0,
                invalid: false,
                content_align: crate::TextHorizontalAlignment::Center,
            }
        );
        let style = context.world().node_style(chip.stable_id()).unwrap();
        assert_eq!(style.layout.border_radius, Some(999.0));
        assert_eq!(
            context
                .world()
                .accessibility(chip.stable_id())
                .unwrap()
                .selected,
            Some(true)
        );
    }

    #[test]
    fn activate_chip_emits_activate_and_skips_when_disabled() {
        let mut context = AppContext::new();
        let chip = context
            .create_component(document(), Chip::new("Plan"))
            .unwrap();
        let fired = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let count = std::sync::Arc::clone(&fired);
        context
            .on(chip, move |_, _: &Activate, _| {
                *count.lock().unwrap() += 1;
            })
            .unwrap();
        assert!(context.activate_chip(chip).unwrap());
        assert_eq!(*fired.lock().unwrap(), 1);
        assert!(
            context.activate_node(chip.stable_id()).unwrap(),
            "Chip must be in builtin activations so pointer hits emit Activate"
        );
        assert_eq!(*fired.lock().unwrap(), 2);

        context
            .update_component(chip, |chip, _| chip.disabled = true)
            .unwrap();
        assert!(!context.activate_chip(chip).unwrap());
        assert!(!context.activate_node(chip.stable_id()).unwrap());
        assert_eq!(*fired.lock().unwrap(), 2);
    }

    #[test]
    fn body_emits_activate_and_close_emits_dismissed_without_crosstalk() {
        let mut context = AppContext::new();
        let chip = context
            .create_component(document(), Chip::new("Plan"))
            .unwrap();
        context
            .update_component(chip, |chip, _| chip.dismissible = true)
            .unwrap();
        let close = Entity::<IconButton>::from_stable_id(
            context
                .read(chip, |chip| chip.close)
                .unwrap()
                .expect("dismissible chip assembles a close button"),
        );
        assert_eq!(context.chip_dismiss_target(close.stable_id()), Some(chip));
        assert_eq!(context.chip_dismiss_target(chip.stable_id()), None);

        let chip_a11y = context.world().accessibility(chip.stable_id()).unwrap();
        assert_eq!(chip_a11y.role, AccessibilityRole::Button);
        assert_ne!(chip_a11y.role, AccessibilityRole::ListItem);
        let close_a11y = context.world().accessibility(close.stable_id()).unwrap();
        assert_eq!(close_a11y.role, AccessibilityRole::Button);
        assert!(
            context
                .world()
                .interaction(close.stable_id())
                .unwrap()
                .focusable
        );

        let activates = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let dismissed = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let activate_count = std::sync::Arc::clone(&activates);
        let dismiss_count = std::sync::Arc::clone(&dismissed);
        context
            .on(chip, move |_, _: &Activate, _| {
                *activate_count.lock().unwrap() += 1;
            })
            .unwrap();
        context
            .on(chip, move |_, _: &ChipDismissed, _| {
                *dismiss_count.lock().unwrap() += 1;
            })
            .unwrap();

        assert!(context.activate_chip(chip).unwrap());
        assert_eq!(*activates.lock().unwrap(), 1);
        assert_eq!(*dismissed.lock().unwrap(), 0);

        assert!(context.activate_icon_button(close).unwrap());
        assert_eq!(*activates.lock().unwrap(), 1);
        assert_eq!(*dismissed.lock().unwrap(), 1);

        context
            .update_component(chip, |chip, _| chip.disabled = true)
            .unwrap();
        assert!(!context.activate_chip(chip).unwrap());
        assert!(!context.activate_icon_button(close).unwrap());
        assert_eq!(*activates.lock().unwrap(), 1);
        assert_eq!(*dismissed.lock().unwrap(), 1);
        assert!(
            !context
                .world()
                .interaction(chip.stable_id())
                .unwrap()
                .focusable
        );
        assert!(
            !context
                .world()
                .interaction(close.stable_id())
                .unwrap()
                .focusable
        );
    }

    #[test]
    fn assemble_chip_adds_close_only_when_dismissible() {
        let mut context = AppContext::new();
        let chip = context
            .create_component(document(), Chip::new("附件").dismissible(true))
            .unwrap();
        assert!(context.assemble_chip(chip).unwrap());
        let close = context.read(chip, |chip| chip.close).unwrap();
        assert!(close.is_some());
        context
            .update_component(chip, |chip, _| chip.dismissible = false)
            .unwrap();
        assert!(!context.assemble_chip(chip).unwrap());
        let hidden = context
            .read(
                Entity::<IconButton>::from_stable_id(close.unwrap()),
                |button| button.style.layout.hidden,
            )
            .unwrap();
        assert!(hidden);
    }
}
