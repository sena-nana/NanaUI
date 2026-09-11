//! Experimental Android slot as a NanaUI Runtime document.
//!
//! Hosts [`RuntimeDocument`] + [`RuntimeInputAdapter`] + host text shaping — the
//! same contract `run_runtime` uses on desktop. The Android Activity still owns
//! the window and event loop; this type does not call `run_runtime` (winit).
//! Soft keyboard show/hide is driven by the host from [`Self::text_input_focused`];
//! printable commits map to [`ImeEvent::Commit`] via
//! [`RuntimeInputAdapter::dispatch_ime`]. NativeActivity has no InputConnection,
//! so there is no composition/preedit. Accessibility name/role/value is the
//! same Runtime projection desktop hosts publish.

#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::sync::{Arc, Mutex};
use std::time::Instant;

use nana_ui::runtime::{
    AccessibilityActionRequest, Activate, Button, DocumentId, Entity, FrameworkError,
    LayoutViewport, List, NodeStyle, RuntimeDocument, Switch, Text, TextChanged, TextInput,
    ToggleChanged,
};
use nana_ui::{AccessibilityNode, NanaTextShaper, RuntimeAnimationClock, RuntimeInputAdapter};
use nana_ui_core::{AlignSpec, FlexDirection, JustifySpec, LengthSpec, PhysicalRect};
use nana_ui_platform::ImeEvent;

use crate::control_slot::{CONTROL_SLOT_INSET, CONTROL_SLOT_LOGICAL_HEIGHT};
use crate::slot_input::{
    SlotInputGate, SlotKeyDispatch, SlotKeyMods, SlotLogicalKey, SlotTouchKind, logical_point,
    pointer_in_slot, slot_key_to_dispatch, touch_to_pointer_event,
};

const SLOT_BUTTON_LABEL: &str = "Nana";
const SLOT_TEXT_LABEL: &str = "Shell";
const SLOT_ICON_GLYPH: &str = "⚙";
const SLOT_SWITCH_LABEL: &str = "On";
const SLOT_INPUT_PLACEHOLDER: &str = "Type…";

#[derive(Debug, Clone, PartialEq, Default)]
struct SlotStripState {
    press_count: u32,
    switch_on: bool,
    input_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlotSnapshot {
    pub press_count: u32,
    pub switch_on: bool,
    pub input_len: usize,
}

/// Retained NanaUI document for the bottom control strip.
pub struct SlotRuntime {
    document: RuntimeDocument,
    shaper: NanaTextShaper,
    adapter: RuntimeInputAdapter,
    clock: RuntimeAnimationClock,
    physical_size: (u32, u32),
    scale: f32,
    state: Arc<Mutex<SlotStripState>>,
    gate: SlotInputGate,
    last_touch_in_slot: bool,
    ime_enabled: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    button: Entity<Button>,
    #[cfg_attr(not(test), allow(dead_code))]
    field: Entity<TextInput>,
}

impl SlotRuntime {
    pub fn new(physical_size: (u32, u32), scale: f32) -> Result<Self, FrameworkError> {
        let document_id = DocumentId::new(1).expect("android slot document id");
        let mut document = RuntimeDocument::new(document_id);
        let state = Arc::new(Mutex::new(SlotStripState::default()));

        let presses = Arc::clone(&state);
        let toggles = Arc::clone(&state);
        let inputs = Arc::clone(&state);
        let (button, field) = document.context_mut().build(document_id, |ui| {
            ui.with("column", column_host(), |ui| {
                ui.with("row", row_strip(), |ui| {
                    ui.child("icon", Text::new(SLOT_ICON_GLYPH));
                    ui.child("caption", Text::new(SLOT_TEXT_LABEL));
                    let field = ui.child("field", slot_text_input());
                    let switch = ui.child("switch", Switch::new(SLOT_SWITCH_LABEL, false));
                    let button = ui.child("button", Button::new(SLOT_BUTTON_LABEL));
                    ui.on(button, move |button, _event: &Activate, _cx| {
                        let mut slot = lock_state(&presses);
                        slot.press_count = slot.press_count.saturating_add(1);
                        button.label = format!("{SLOT_BUTTON_LABEL} · {}", slot.press_count);
                    });
                    ui.on(switch, move |switch, event: &ToggleChanged, _cx| {
                        switch.checked = event.checked;
                        lock_state(&toggles).switch_on = event.checked;
                    });
                    ui.on(field, move |field, event: &TextChanged, _cx| {
                        field.state.replace_value(event.value.clone());
                        lock_state(&inputs).input_value = event.value.clone();
                    });
                    (button, field)
                })
            })
        })?;

        let mut runtime = Self {
            document,
            shaper: NanaTextShaper::default(),
            adapter: RuntimeInputAdapter::default(),
            clock: RuntimeAnimationClock::now(),
            physical_size: (physical_size.0.max(1), physical_size.1.max(1)),
            scale: scale.max(0.25),
            state,
            gate: SlotInputGate::default(),
            last_touch_in_slot: false,
            ime_enabled: false,
            button,
            field,
        };
        runtime.flush()?;
        Ok(runtime)
    }

    pub fn resize(&mut self, physical_size: (u32, u32), scale: f32) {
        self.physical_size = (physical_size.0.max(1), physical_size.1.max(1));
        self.scale = scale.max(0.25);
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn physical_size(&self) -> (u32, u32) {
        self.physical_size
    }

    pub fn press_count(&self) -> u32 {
        lock_state(&self.state).press_count
    }

    pub fn switch_on(&self) -> bool {
        lock_state(&self.state).switch_on
    }

    pub fn input_value(&self) -> String {
        lock_state(&self.state).input_value.clone()
    }

    pub fn last_touch_in_slot(&self) -> bool {
        self.last_touch_in_slot
    }

    /// Whether the Runtime keyboard focus sits on the slot's text input.
    ///
    /// The Android host mirrors this into `show_soft_input` / `hide_soft_input`
    /// so tapping the field raises the soft keyboard and moving focus away
    /// lowers it.
    pub fn text_input_focused(&self) -> bool {
        self.document
            .context()
            .world()
            .focused(self.document.document())
            == Some(self.field.stable_id())
    }

    pub(crate) fn snapshot(&self) -> SlotSnapshot {
        SlotSnapshot {
            press_count: self.press_count(),
            switch_on: self.switch_on(),
            input_len: self.input_value().len(),
        }
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub fn flush(&mut self) -> Result<(), FrameworkError> {
        let (logical_w, logical_h) = self.logical_size();
        self.document
            .flush(LayoutViewport::new(logical_w, logical_h), &mut self.shaper)?;
        Ok(())
    }

    pub(crate) fn logical_size(&self) -> (f32, f32) {
        (
            self.physical_size.0 as f32 / self.scale,
            self.physical_size.1 as f32 / self.scale,
        )
    }

    /// Queue a physical pointer sample (Android MotionEvent coords + pointer id).
    ///
    /// Returns `false` when the sample is outside the slot (and not part of
    /// an in-slot drag) so the host can leave it `Unhandled` for VueHost.
    pub fn push_touch(
        &mut self,
        slot: Option<PhysicalRect>,
        kind: SlotTouchKind,
        physical_x: f32,
        physical_y: f32,
        pointer_id: i32,
    ) -> Result<bool, FrameworkError> {
        self.last_touch_in_slot = slot
            .map(|rect| pointer_in_slot(rect, physical_x, physical_y))
            .unwrap_or(false);
        if !self
            .gate
            .accept_pointer(slot, kind, physical_x, physical_y, pointer_id)
        {
            return Ok(false);
        }
        if kind == SlotTouchKind::Down && !self.pointer_hits_text_input(physical_x, physical_y) {
            // Disable IME while the field still has Runtime focus so leftover
            // preedit commits through the desktop Disabled path.
            self.commit_ime_on_blur()?;
        }
        let logical = logical_point(physical_x, physical_y, self.scale);
        let event = touch_to_pointer_event(
            kind,
            logical,
            pointer_id,
            nana_ui_platform::InputModifiers::default(),
        );
        self.dispatch(&event)?;
        self.sync_ime_lifecycle()?;
        Ok(true)
    }

    /// Queue a keyboard sample (Android KeyEvent → Runtime).
    ///
    /// Returns `false` when the slot does not hold keyboard focus so the host
    /// does not swallow whole-window keys. Printable soft-keyboard commits
    /// become [`ImeEvent::Commit`] while the text input is focused; editing
    /// keys stay on the keyboard path. When `key` is `None`, only modifier
    /// state is recorded.
    pub fn push_key(
        &mut self,
        down: bool,
        key: Option<SlotLogicalKey>,
        mods: SlotKeyMods,
        repeat: bool,
    ) -> Result<bool, FrameworkError> {
        if !self.gate.accept_key() {
            return Ok(false);
        }
        let Some(key) = key else {
            return Ok(true);
        };
        let focused = self.text_input_focused();
        match slot_key_to_dispatch(down, key, mods, repeat, focused) {
            SlotKeyDispatch::Keyboard(event) => self.dispatch(&event)?,
            SlotKeyDispatch::Ime(event) => self.dispatch_ime_event(&event)?,
        }
        self.sync_ime_lifecycle()?;
        Ok(true)
    }

    /// Inject a desktop IME event into the focused Runtime editor.
    ///
    /// NativeActivity never synthesizes Preedit; tests (and a future
    /// InputConnection host) call this so composition still uses
    /// [`RuntimeInputAdapter::dispatch_ime`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn push_ime(&mut self, event: &ImeEvent) -> Result<bool, FrameworkError> {
        if !self.gate.accept_key() {
            return Ok(false);
        }
        self.dispatch_ime_event(event)?;
        self.sync_ime_lifecycle()?;
        Ok(true)
    }

    /// Runtime accessibility nodes for the slot tree (name/role/value/focus).
    pub fn accessibility_nodes(&self) -> Vec<AccessibilityNode> {
        let document = self.document();
        document
            .context()
            .world()
            .project_accessibility(document.document())
    }

    /// Apply a typed accessibility action and republish IME focus.
    pub fn apply_accessibility_action(
        &mut self,
        request: AccessibilityActionRequest,
    ) -> Result<bool, FrameworkError> {
        if request.target != self.field.stable_id() {
            self.commit_ime_on_blur()?;
        }
        let document = self.document.document();
        let applied = self
            .document
            .context_mut()
            .apply_accessibility_action(document, request)?;
        self.flush()?;
        self.sync_ime_lifecycle()?;
        Ok(applied)
    }

    fn dispatch(&mut self, event: &nana_ui_platform::InputEvent) -> Result<(), FrameworkError> {
        let document_id = self.document.document();
        let now = self.clock.runtime_time(Instant::now());
        self.adapter
            .dispatch_at(self.document.context_mut(), document_id, event, now)?;
        self.flush()
    }

    fn dispatch_ime_event(&mut self, event: &ImeEvent) -> Result<(), FrameworkError> {
        let document_id = self.document.document();
        self.adapter
            .dispatch_ime(self.document.context_mut(), document_id, event)?;
        self.flush()
    }

    fn sync_ime_lifecycle(&mut self) -> Result<(), FrameworkError> {
        let focused = self.text_input_focused();
        if focused && !self.ime_enabled {
            self.ime_enabled = true;
            self.dispatch_ime_event(&ImeEvent::Enabled)?;
        } else if !focused && self.ime_enabled {
            self.commit_ime_on_blur()?;
        }
        Ok(())
    }

    fn commit_ime_on_blur(&mut self) -> Result<(), FrameworkError> {
        if !self.ime_enabled || !self.text_input_focused() {
            self.ime_enabled = false;
            return Ok(());
        }
        self.ime_enabled = false;
        self.dispatch_ime_event(&ImeEvent::Disabled)
    }

    fn pointer_hits_text_input(&self, physical_x: f32, physical_y: f32) -> bool {
        let Some(layout) = self
            .document
            .context()
            .world()
            .layout_box(self.field.stable_id())
        else {
            return false;
        };
        let [x, y] = logical_point(physical_x, physical_y, self.scale);
        x >= layout.x
            && x < layout.x + layout.width
            && y >= layout.y
            && y < layout.y + layout.height
    }
}

fn lock_state(state: &Arc<Mutex<SlotStripState>>) -> std::sync::MutexGuard<'_, SlotStripState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn column_host() -> List {
    let mut list = List::new().label("Slot");
    let layout = std::sync::Arc::make_mut(&mut list.style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.justify_content = JustifySpec::End;
    layout.padding_left = Some(LengthSpec::Px(CONTROL_SLOT_INSET));
    layout.padding_right = Some(LengthSpec::Px(CONTROL_SLOT_INSET));
    layout.padding_bottom = Some(LengthSpec::Px(CONTROL_SLOT_INSET));
    list
}

fn row_strip() -> List {
    let mut list = List::new();
    list.style = NodeStyle::visible();
    let layout = std::sync::Arc::make_mut(&mut list.style.layout);
    layout.direction = Some(FlexDirection::Row);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Px(CONTROL_SLOT_LOGICAL_HEIGHT));
    layout.align_items = AlignSpec::Center;
    layout.gap = Some(LengthSpec::Px(10.0));
    layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
    list
}

fn slot_text_input() -> TextInput {
    let mut field = TextInput::new("").placeholder(SLOT_INPUT_PLACEHOLDER);
    let layout = std::sync::Arc::make_mut(&mut field.style.layout);
    layout.width = Some(LengthSpec::Px(120.0));
    layout.min_width = Some(LengthSpec::Px(64.0));
    layout.flex_shrink = Some(1.0);
    field
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_slot::control_slot_paint_bounds;
    use nana_ui::runtime::{AccessibilityAction, AccessibilityRole};

    fn runtime() -> SlotRuntime {
        SlotRuntime::new((1080, 1920), 2.0).expect("slot runtime")
    }

    fn field_id(slot: &SlotRuntime) -> nana_ui::runtime::StableNodeId {
        slot.field.stable_id()
    }

    fn button_id(slot: &SlotRuntime) -> nana_ui::runtime::StableNodeId {
        slot.button.stable_id()
    }

    fn tap_entity(slot: &mut SlotRuntime, id: nana_ui::runtime::StableNodeId) {
        let layout = slot
            .document()
            .context()
            .world()
            .layout_box(id)
            .expect("layout");
        let scale = slot.scale();
        let x = (layout.x + layout.width * 0.5) * scale;
        let y = (layout.y + layout.height * 0.5) * scale;
        let bounds = control_slot_paint_bounds(slot.physical_size(), scale);
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Down, x, y, 0)
                .expect("down")
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Up, x, y, 0)
                .expect("up")
        );
    }

    fn node_with_role(nodes: &[AccessibilityNode], role: AccessibilityRole) -> &AccessibilityNode {
        nodes
            .iter()
            .find(|node| node.role == role)
            .unwrap_or_else(|| panic!("missing {role:?}"))
    }

    #[test]
    fn flush_extracts_scene_primitives() {
        let slot = runtime();
        assert!(slot.document().scene().primitives().count() > 0);
        assert_eq!(slot.press_count(), 0);
        assert!(!slot.switch_on());
        assert!(slot.input_value().is_empty());
    }

    #[test]
    fn pointer_activate_increments_button() {
        let mut slot = runtime();
        let layout = slot
            .document()
            .context()
            .world()
            .layout_box(slot.button.stable_id())
            .expect("button layout");
        assert!(layout.width > 0.0 && layout.height > 0.0);
        let scale = slot.scale();
        let x = (layout.x + layout.width * 0.5) * scale;
        let y = (layout.y + layout.height * 0.5) * scale;
        let bounds = control_slot_paint_bounds(slot.physical_size(), scale);
        let slot_rect = bounds.expect("slot bounds");
        assert!(
            crate::slot_input::pointer_in_slot(slot_rect, x, y),
            "button center ({x}, {y}) layout={layout:?} must sit in slot {slot_rect:?}"
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Down, x, y, 0)
                .expect("down")
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Up, x, y, 0)
                .expect("up")
        );
        assert_eq!(slot.press_count(), 1);
        assert!(slot.last_touch_in_slot());
    }

    #[test]
    fn focused_key_commits_text_input() {
        let mut slot = runtime();
        let layout = slot
            .document()
            .context()
            .world()
            .layout_box(slot.field.stable_id())
            .expect("field layout");
        assert!(layout.width > 0.0 && layout.height > 0.0);
        let scale = slot.scale();
        let x = (layout.x + layout.width * 0.5) * scale;
        let y = (layout.y + layout.height * 0.5) * scale;
        let bounds = control_slot_paint_bounds(slot.physical_size(), scale);
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Down, x, y, 0)
                .expect("focus down")
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Up, x, y, 0)
                .expect("focus up")
        );
        assert!(
            slot.push_key(
                true,
                Some(SlotLogicalKey::Character('h')),
                SlotKeyMods::default(),
                false,
            )
            .expect("type")
        );
        assert_eq!(slot.input_value(), "h");
    }

    #[test]
    fn text_input_focus_tracks_taps_for_soft_input_mirror() {
        let mut slot = runtime();
        let bounds = control_slot_paint_bounds(slot.physical_size(), slot.scale());
        let field_center = |slot: &SlotRuntime| {
            let layout = slot
                .document()
                .context()
                .world()
                .layout_box(slot.field.stable_id())
                .expect("field layout");
            let scale = slot.scale();
            (
                (layout.x + layout.width * 0.5) * scale,
                (layout.y + layout.height * 0.5) * scale,
            )
        };
        let button_center = |slot: &SlotRuntime| {
            let layout = slot
                .document()
                .context()
                .world()
                .layout_box(slot.button.stable_id())
                .expect("button layout");
            let scale = slot.scale();
            (
                (layout.x + layout.width * 0.5) * scale,
                (layout.y + layout.height * 0.5) * scale,
            )
        };
        assert!(!slot.text_input_focused());

        let (fx, fy) = field_center(&slot);
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Down, fx, fy, 0)
                .expect("field down")
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Up, fx, fy, 0)
                .expect("field up")
        );
        assert!(
            slot.text_input_focused(),
            "tapping the field must move Runtime focus onto the text input"
        );

        let (bx, by) = button_center(&slot);
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Down, bx, by, 0)
                .expect("button down")
        );
        assert!(
            slot.push_touch(bounds, SlotTouchKind::Up, bx, by, 0)
                .expect("button up")
        );
        assert!(
            !slot.text_input_focused(),
            "tapping another control must drop text-input focus so the host hides the keyboard"
        );
    }

    #[test]
    fn pointer_outside_slot_is_not_handled() {
        let mut slot = runtime();
        let bounds = control_slot_paint_bounds(slot.physical_size(), slot.scale());
        assert!(
            !slot
                .push_touch(bounds, SlotTouchKind::Down, 10.0, 10.0, 0)
                .expect("outside")
        );
        assert_eq!(slot.press_count(), 0);
        assert!(!slot.last_touch_in_slot());
    }

    #[test]
    fn ime_commit_writes_cjk_without_keycode_table() {
        let mut slot = runtime();
        let field = field_id(&slot);
        tap_entity(&mut slot, field);
        assert!(slot.text_input_focused());
        assert!(
            slot.push_ime(&ImeEvent::Commit("你好".into()))
                .expect("commit")
        );
        assert_eq!(slot.input_value(), "你好");
    }

    #[test]
    fn ime_preedit_then_commit_uses_desktop_composition_path() {
        let mut slot = runtime();
        let field = field_id(&slot);
        tap_entity(&mut slot, field);
        assert!(
            slot.push_ime(&ImeEvent::Preedit {
                text: "你".into(),
                selection: Some((0, "你".len())),
            })
            .expect("preedit")
        );
        assert!(
            slot.input_value().is_empty(),
            "preedit must not commit the editor value"
        );
        assert!(
            slot.push_ime(&ImeEvent::Commit("你好".into()))
                .expect("commit")
        );
        assert_eq!(slot.input_value(), "你好");
    }

    #[test]
    fn leaving_text_input_commits_leftover_preedit() {
        let mut slot = runtime();
        let field = field_id(&slot);
        tap_entity(&mut slot, field);
        assert!(
            slot.push_ime(&ImeEvent::Preedit {
                text: "世".into(),
                selection: Some((0, "世".len())),
            })
            .expect("preedit")
        );
        let button = button_id(&slot);
        tap_entity(&mut slot, button);
        assert!(
            !slot.text_input_focused(),
            "leaving the field must disable IME"
        );
        assert_eq!(slot.input_value(), "世");
    }

    #[test]
    fn accessibility_nodes_publish_slot_name_role_value() {
        let mut slot = runtime();
        let nodes = slot.accessibility_nodes();
        assert!(
            nodes.iter().any(|node| node.parent.is_none()),
            "slot tree must publish a root"
        );
        let button = node_with_role(&nodes, AccessibilityRole::Button);
        assert_eq!(button.label.as_deref(), Some(SLOT_BUTTON_LABEL));
        let switch = node_with_role(&nodes, AccessibilityRole::Switch);
        assert_eq!(switch.label.as_deref(), Some(SLOT_SWITCH_LABEL));
        let field = node_with_role(&nodes, AccessibilityRole::TextInput);
        assert!(!field.focused);
        let field = field_id(&slot);
        tap_entity(&mut slot, field);
        let nodes = slot.accessibility_nodes();
        let field = node_with_role(&nodes, AccessibilityRole::TextInput);
        assert!(field.focused);
        assert!(
            slot.push_key(
                true,
                Some(SlotLogicalKey::Character('h')),
                SlotKeyMods::default(),
                false,
            )
            .expect("type")
        );
        let nodes = slot.accessibility_nodes();
        let field = node_with_role(&nodes, AccessibilityRole::TextInput);
        assert_eq!(field.value.as_deref(), Some("h"));
    }

    #[test]
    fn accessibility_click_activates_slot_button() {
        let mut slot = runtime();
        let nodes = slot.accessibility_nodes();
        let target = node_with_role(&nodes, AccessibilityRole::Button).id;
        assert!(
            slot.apply_accessibility_action(AccessibilityActionRequest {
                target,
                action: AccessibilityAction::Click,
            })
            .expect("click")
        );
        assert_eq!(slot.press_count(), 1);
    }
}
