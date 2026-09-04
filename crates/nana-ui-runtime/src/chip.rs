//! Compact selectable token. Visual language is a pill `Button`; dismiss is a
//! trailing close control assembled by [`AppContext::assemble_chip`].

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ButtonKind, ControlSize, FlexDirection, Icon, LengthSpec, SemanticColorRole,
    UI_METRICS,
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
    pub selected: bool,
    pub disabled: bool,
    pub dismissible: bool,
    pub close_label: Arc<str>,
    pub size: ControlSize,
    pub style: NodeStyle,
    pub(crate) close: Option<StableNodeId>,
}

impl Chip {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            disabled: false,
            dismissible: false,
            close_label: Arc::from("移除"),
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
        self.close_label = close_label.into();
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
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.gap = Some(LengthSpec::Px(4.0));
        layout.padding_left = Some(LengthSpec::Px(UI_METRICS.compact_control_padding_x));
        layout.padding_right = layout.padding_left;
        layout.min_height = Some(LengthSpec::Px(UI_METRICS.compact_control_height));
        layout.border_width = Some(1.0);
        layout.border_radius = Some(999.0);
        layout.font_size = Some(self.size.text_size());
        layout.font_weight = Some(500);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for Chip {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "chip".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let text = TextContent {
            value: self.label.to_string(),
        };
        if world.text(id) != Some(text.value.as_str()) {
            mutations.set_text(id, text);
        }
        let visual = StandardVisual::Button {
            label: Arc::clone(&self.label),
            kind: self.kind(),
            size: self.size,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
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
                label: Some(Arc::clone(&self.label)),
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

        let created = snapshot.close.is_none();
        let close = match snapshot.close.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<IconButton>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                IconButton::new(Icon::Close, snapshot.close_label.as_ref())
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
            button.label = Arc::clone(&snapshot.close_label);
            Arc::make_mut(&mut button.style.layout).hidden = false;
        })?;
        self.update_component(chip, |chip, _| {
            chip.close = Some(close.stable_id());
        })?;
        self.append_child(chip, close)?;
        Ok(created)
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
                label: Arc::from("Beta"),
                kind: ButtonKind::Selected,
                size: ControlSize::Small,
                loading: false,
                loading_phase: 0.0,
                invalid: false,
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
