//! Press-and-hold pointer/touch primitive ([`HoldPad`] and [`PressChange`]).
//!
//! Emits [`PressChange { pressed: true }`] immediately on primary pointer/touch
//! down and [`PressChange { pressed: false }`] on release, pointer cancel, or
//! deactivation — suitable for push-to-talk, hold-to-trigger mobile pads, and
//! momentary live controls without waiting for click completion.

use std::sync::Arc;

use nana_ui_core::{LengthSpec, RadiusTier, SemanticColorRole};

use super::controls::StyledComponent;
use super::node::{El, widget};
use super::prop::{FieldWrite, IntoProp};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, InteractionState,
    MutationQueue, NodeKind, NodePointer, NodePointerHooks, NodeStyle, StableNodeId, UiWorld,
};

/// Emitted by [`HoldPad`] when its press-and-hold state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressChange {
    pub pressed: bool,
}

/// Interactive press-and-hold surface driven by [`NodePointerHooks`].
#[derive(Debug, Clone, PartialEq)]
pub struct HoldPad {
    pub label: Arc<str>,
    pub disabled: bool,
    pub pressed: bool,
    pub style: NodeStyle,
}

impl StyledComponent for HoldPad {
    fn node_style(&self) -> &NodeStyle {
        &self.style
    }

    fn node_style_mut(&mut self) -> &mut NodeStyle {
        &mut self.style
    }
}

impl HoldPad {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        let mut style = NodeStyle::default();
        style.interaction.hovered.background = Some(SemanticColorRole::Hover);
        style.interaction.pressed.background = Some(SemanticColorRole::Active);
        Arc::make_mut(&mut style.layout).theme.border_radius =
            Some(nana_ui_core::ThemeToken::Radius(RadiusTier::Md).into());
        Self {
            label: label.into(),
            disabled: false,
            pressed: false,
            style,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        if disabled {
            self.pressed = false;
        }
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_layout(mut self, f: impl FnOnce(&mut nana_ui_core::LayoutStyle)) -> Self {
        f(Arc::make_mut(&mut self.style.layout));
        self
    }

    pub fn width(self, width: LengthSpec) -> Self {
        self.with_layout(|l| l.width = Some(width))
    }

    pub fn height(self, height: LengthSpec) -> Self {
        self.with_layout(|l| l.height = Some(height))
    }
}

fn hold_pad_pressed(
    cx: &mut AppContext,
    entity: Entity<HoldPad>,
    pointer: NodePointer,
) -> Result<bool, crate::FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    cx.update_component(entity, |pad, view_cx| {
        if pad.disabled {
            return false;
        }
        if !pad.pressed {
            pad.pressed = true;
            view_cx.emit(PressChange { pressed: true });
        }
        true
    })
}

fn hold_pad_released(
    cx: &mut AppContext,
    entity: Entity<HoldPad>,
    _pointer: NodePointer,
) -> Result<bool, crate::FrameworkError> {
    cx.update_component(entity, |pad, view_cx| {
        if pad.pressed {
            pad.pressed = false;
            view_cx.emit(PressChange { pressed: false });
            true
        } else {
            false
        }
    })
}

fn hold_pad_left(
    cx: &mut AppContext,
    entity: Entity<HoldPad>,
) -> Result<(), crate::FrameworkError> {
    cx.update_component(entity, |pad, view_cx| {
        if pad.pressed {
            pad.pressed = false;
            view_cx.emit(PressChange { pressed: false });
        }
    })
}

static HOLD_PAD_POINTER: NodePointerHooks<HoldPad> = NodePointerHooks {
    moved: |_, _, _| Ok(false),
    left: hold_pad_left,
    pressed: hold_pad_pressed,
    released: hold_pad_released,
    wheel: |_, _, _| Ok(false),
};

impl ComponentView for HoldPad {
    const TYPE_ID: Option<&'static str> = None;

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&HOLD_PAD_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn reconcile(&mut self, mut next: Self) {
        if next.disabled {
            next.pressed = false;
        } else {
            next.pressed = self.pressed;
        }
        *self = next;
    }

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "hold-pad".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        crate::view_components::project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

struct HoldPadLabel;
impl FieldWrite<HoldPad, Arc<str>> for HoldPadLabel {
    const FIELD: &'static str = "HoldPad.label";
    fn write(target: &mut HoldPad, value: Arc<str>) {
        target.label = value;
    }
    fn differs(target: &HoldPad, value: &Arc<str>) -> bool {
        target.label != *value
    }
}

struct HoldPadDisabled;
impl FieldWrite<HoldPad, bool> for HoldPadDisabled {
    const FIELD: &'static str = "HoldPad.disabled";
    fn write(target: &mut HoldPad, value: bool) {
        target.disabled = value;
        if value {
            target.pressed = false;
        }
    }
    fn differs(target: &HoldPad, value: &bool) -> bool {
        target.disabled != *value
    }
}

/// Declarative [`HoldPad`] builder.
#[track_caller]
pub fn hold_pad(label: impl IntoProp<Arc<str>>) -> El<HoldPad> {
    widget(HoldPad::new("")).prop::<Arc<str>, HoldPadLabel>(label)
}

impl<K> El<HoldPad, K> {
    #[track_caller]
    pub fn disabled(self, disabled: impl IntoProp<bool>) -> Self {
        self.prop::<bool, HoldPadDisabled>(disabled)
    }

    /// Observe press-and-hold transitions (`true` on press down, `false` on
    /// release or cancel).
    pub fn on_press_change(self, mut handler: impl FnMut(bool) + Send + 'static) -> Self {
        self.on(move |event: &PressChange| handler(event.pressed))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::view::IntoView;
    use crate::{AppContext, DocumentId, LayoutViewport};
    use nana_ui_input::{InputModifiers, PointerInput, PointerPhase};

    #[test]
    fn hold_pad_emits_press_down_release_and_cancel() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let log: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let log_cb = Arc::clone(&log);

        let view = cx
            .mount_view_root(document, move || {
                widget(
                    HoldPad::new("Push to talk")
                        .width(LengthSpec::Px(120.0))
                        .height(LengthSpec::Px(48.0)),
                )
                .on_press_change(move |pressed| log_cb.lock().unwrap().push(pressed))
                .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(200.0, 200.0))
            .unwrap();
        cx.rebuild_hit_test(document);

        let id = view.roots()[0];
        let mut input = crate::HeadlessInput::bind(&mut cx, document);
        let mut down = PointerInput::mouse(PointerPhase::Down, 40.0, 20.0);
        down.button = 0;
        down.buttons = 1;
        down.modifiers = InputModifiers::default();
        input
            .route(&mut cx, nana_ui_input::InputPayload::Pointer(down))
            .unwrap();
        assert_eq!(*log.lock().unwrap(), vec![true]);

        let mut up = PointerInput::mouse(PointerPhase::Up, 40.0, 20.0);
        up.button = 0;
        up.buttons = 0;
        input
            .route(&mut cx, nana_ui_input::InputPayload::Pointer(up))
            .unwrap();
        assert_eq!(*log.lock().unwrap(), vec![true, false]);

        // Press again and cancel -> must emit false.
        input
            .route(&mut cx, nana_ui_input::InputPayload::Pointer(down))
            .unwrap();
        assert_eq!(*log.lock().unwrap(), vec![true, false, true]);
        let cancel = PointerInput::mouse(PointerPhase::Cancel, 40.0, 20.0);
        input
            .route(&mut cx, nana_ui_input::InputPayload::Pointer(cancel))
            .unwrap();
        assert_eq!(*log.lock().unwrap(), vec![true, false, true, false]);
        let _ = id;
    }
}
