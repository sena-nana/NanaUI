//! Backend-neutral lowering helpers and stateful trackers for host adapters.
//!
//! Windowing hosts (such as `winit` or embedded platform loops) map their own
//! native event enums onto [`PointerState`], [`PenFramePatch`], and
//! [`DragTransferTracker`] here so pointer button masks, screen-space scaling,
//! Win32 pen pressure/cancel fixes, and OS drag-transfer staging live once in
//! `nana-ui-input` without pulling any windowing dependency into this crate.

use std::path::PathBuf;

use nana_ui_core::FileDragKind;

use crate::{
    DeviceId, FileDragInput, InputModifiers, InputPayload, PointerId, PointerInput, PointerPhase,
    PointerType,
};

/// W3C code of the primary mouse button, whose drag semantics follow the
/// platform's native window drag.
pub const PRIMARY_MOUSE_BUTTON: i16 = 0;

/// Bitmask bit for a W3C mouse button code (`0..=4`).
pub const fn mouse_button_mask(button: i16) -> u16 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}

/// The first held mouse button in `buttons`, primary button first.
pub fn held_mouse_button(buttons: u16) -> Option<i16> {
    [PRIMARY_MOUSE_BUTTON, 1, 2, 3, 4]
        .into_iter()
        .find(|&button| buttons & mouse_button_mask(button) != 0)
}

/// Shift turns a vertical wheel delta horizontal on platforms that do not
/// already swap the axis before delivering the event.
pub const fn shift_wheel_axes(delta: (f32, f32), shift: bool, platform_swaps: bool) -> (f32, f32) {
    if shift && !platform_swaps {
        (delta.1, delta.0)
    } else {
        delta
    }
}

/// Where a window's client area sits in desktop-logical space, and the ratio
/// taking content-logical client points into that desktop space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenSpace {
    pub origin: (f32, f32),
    pub client_ratio: f32,
}

/// Converts a client-logical point into desktop-logical coordinates when a
/// [`ScreenSpace`] is available, falling back to `client` otherwise.
pub fn screen_position(space: Option<ScreenSpace>, client: (f32, f32)) -> (f32, f32) {
    space.map_or(client, |space| {
        (
            space.origin.0 + client.0 * space.client_ratio,
            space.origin.1 + client.1 * space.client_ratio,
        )
    })
}

/// Identity and stylus axes for a pointer device mapped into canonical space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerDescriptor {
    pub pointer_id: u64,
    pub pointer_type: PointerType,
    pub is_primary: bool,
    pub pressure: Option<f32>,
    pub tangential_pressure: f32,
    pub tilt_x: i16,
    pub tilt_y: i16,
    pub twist: u16,
}

impl PointerDescriptor {
    /// Mouse, touch, or non-tablet pointer descriptor without tilt/twist axes.
    pub const fn simple(
        pointer_id: u64,
        pointer_type: PointerType,
        is_primary: bool,
        pressure: Option<f32>,
    ) -> Self {
        Self {
            pointer_id,
            pointer_type,
            is_primary,
            pressure,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
        }
    }
}

/// Pure per-window pointer, button, modifier, and last-device state shared by
/// host input adapters.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PointerState {
    pub cursor: (f32, f32),
    /// Device of the last pointer or key event, for window-level events (e.g.
    /// focus or synthetic pen leave) that carry none.
    pub last_device: DeviceId,
    pub buttons: u16,
    pub modifiers: InputModifiers,
}

impl PointerState {
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
    }

    pub fn set_modifiers(&mut self, modifiers: InputModifiers) {
        self.modifiers = modifiers;
    }

    pub fn clear_buttons(&mut self) {
        self.buttons = 0;
    }

    /// Updates held button mask for `button_code` and returns the new bitmask.
    pub fn set_button_pressed(&mut self, button_code: i16, pressed: bool) -> u16 {
        let mask = mouse_button_mask(button_code);
        if pressed {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.buttons
    }

    /// Builds a [`PointerInput`] at [`Self::cursor`] with the given phase,
    /// button mask, modifiers, and optional screen mapping.
    #[expect(
        clippy::too_many_arguments,
        reason = "Direct projection onto canonical PointerInput fields"
    )]
    pub fn pointer_event(
        &self,
        descriptor: PointerDescriptor,
        phase: PointerPhase,
        button: i16,
        buttons: u16,
        activation_click: bool,
        modifiers: InputModifiers,
        screen_origin: Option<ScreenSpace>,
        pressure: Option<f32>,
    ) -> PointerInput {
        let screen = screen_position(screen_origin, self.cursor);
        PointerInput {
            phase,
            pointer_id: PointerId(descriptor.pointer_id),
            pointer_type: descriptor.pointer_type,
            x: self.cursor.0,
            y: self.cursor.1,
            screen_x: screen.0,
            screen_y: screen.1,
            button,
            buttons,
            pressure: pressure.unwrap_or_else(|| {
                descriptor
                    .pressure
                    .unwrap_or(if buttons == 0 { 0.0 } else { 0.5 })
            }),
            tangential_pressure: descriptor.tangential_pressure,
            tilt_x: descriptor.tilt_x,
            tilt_y: descriptor.tilt_y,
            twist: descriptor.twist,
            is_primary: descriptor.is_primary,
            activation_click,
            modifiers,
        }
    }

    /// Ends an active mouse gesture whose release the platform will not deliver.
    pub fn cancel_mouse(&mut self, screen_origin: Option<ScreenSpace>) -> PointerInput {
        let buttons = std::mem::take(&mut self.buttons);
        self.pointer_event(
            PointerDescriptor::simple(1, PointerType::Mouse, true, None),
            PointerPhase::Cancel,
            -1,
            buttons,
            false,
            self.modifiers,
            screen_origin,
            Some(0.0),
        )
    }
}

/// Corrections reported by a native window procedure for the active pen frame:
/// a cancelled lift becomes [`PointerPhase::Cancel`], and an explicit pressure
/// (including `0.0`) overrides the generic pressed default.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PenFramePatch {
    pub pressure: Option<f32>,
    pub canceled: bool,
}

impl PenFramePatch {
    pub fn apply_to(self, payload: &mut InputPayload) {
        let InputPayload::Pointer(pointer) = payload else {
            return;
        };
        if pointer.pointer_type != PointerType::Pen {
            return;
        }
        if self.canceled && pointer.phase == PointerPhase::Up {
            pointer.phase = PointerPhase::Cancel;
        }
        if pointer.buttons != 0
            && let Some(pressure) = self.pressure
        {
            pointer.pressure = pressure;
        }
    }
}

/// Stateful OS file-drag staging shared across asynchronous path fetches and
/// hover/drop/leave window events.
#[derive(Debug)]
pub struct DragTransferTracker<T = u64, S = u64> {
    pub pending_file_paths: Vec<PathBuf>,
    pub file_drop_emitted: bool,
    pub pending_dnd: Option<T>,
    pub pending_dnd_serial: Option<S>,
    pub drop_waiting_for_data: bool,
    /// Keys held at release while its paths are still being fetched.
    pub drop_modifiers: Option<InputModifiers>,
}

impl<T, S> Default for DragTransferTracker<T, S> {
    fn default() -> Self {
        Self {
            pending_file_paths: Vec::new(),
            file_drop_emitted: false,
            pending_dnd: None,
            pending_dnd_serial: None,
            drop_waiting_for_data: false,
            drop_modifiers: None,
        }
    }
}

impl<T: Copy + Eq, S: Copy + Eq> DragTransferTracker<T, S> {
    pub fn reset(&mut self) {
        self.pending_file_paths.clear();
        self.file_drop_emitted = false;
        self.drop_waiting_for_data = false;
        self.drop_modifiers = None;
        self.pending_dnd = None;
        self.pending_dnd_serial = None;
    }

    pub fn has_pending_paths(&self) -> bool {
        !self.pending_file_paths.is_empty()
    }

    pub fn begin(&mut self, transfer: T, serial: Option<S>) {
        self.reset();
        self.pending_dnd = Some(transfer);
        self.pending_dnd_serial = serial;
    }

    pub fn hover(&self, cursor: (f32, f32), modifiers: InputModifiers) -> FileDragInput {
        FileDragInput {
            kind: FileDragKind::Hover,
            paths: self.pending_file_paths.clone(),
            position: Some(cursor),
            modifiers,
        }
    }

    pub fn drop_now(&mut self, cursor: (f32, f32), modifiers: InputModifiers) -> FileDragInput {
        self.file_drop_emitted = true;
        self.drop_waiting_for_data = false;
        self.pending_dnd = None;
        self.pending_dnd_serial = None;
        FileDragInput {
            kind: FileDragKind::Drop,
            paths: std::mem::take(&mut self.pending_file_paths),
            position: Some(cursor),
            modifiers,
        }
    }

    pub fn on_entered(
        &mut self,
        transfer: T,
        cursor: (f32, f32),
        modifiers: InputModifiers,
    ) -> FileDragInput {
        if self.pending_dnd != Some(transfer) {
            self.begin(transfer, None);
        }
        self.hover(cursor, modifiers)
    }

    pub fn on_position(
        &self,
        transfer: T,
        cursor: (f32, f32),
        modifiers: InputModifiers,
    ) -> Option<FileDragInput> {
        if self.pending_dnd != Some(transfer) || self.file_drop_emitted {
            return None;
        }
        Some(self.hover(cursor, modifiers))
    }

    pub fn on_left(&mut self) -> FileDragInput {
        self.reset();
        FileDragInput::cancel()
    }

    pub fn on_dropped(
        &mut self,
        cursor: (f32, f32),
        modifiers: InputModifiers,
    ) -> Option<FileDragInput> {
        if self.file_drop_emitted {
            return None;
        }
        Some(self.drop_now(cursor, modifiers))
    }

    pub fn wait_for_drop_data(&mut self, transfer: T, serial: S, modifiers: InputModifiers) {
        self.pending_dnd = Some(transfer);
        self.pending_dnd_serial = Some(serial);
        self.drop_waiting_for_data = true;
        self.drop_modifiers = Some(modifiers);
    }

    pub fn abandon_drop(&mut self, transfer: T) -> Option<FileDragInput> {
        if self.pending_dnd != Some(transfer) || !self.drop_waiting_for_data {
            return None;
        }
        self.reset();
        self.file_drop_emitted = true;
        Some(FileDragInput::cancel())
    }

    pub fn accepts_serial(&self, transfer: T, serial: S) -> bool {
        self.pending_dnd == Some(transfer)
            && self
                .pending_dnd_serial
                .is_none_or(|pending| pending == serial)
    }

    pub fn ingest_paths(
        &mut self,
        transfer: T,
        paths: Vec<PathBuf>,
        cursor: (f32, f32),
        fallback_modifiers: InputModifiers,
    ) -> Option<FileDragInput> {
        if self.pending_dnd != Some(transfer) {
            return None;
        }
        self.pending_file_paths = paths;
        if self.drop_waiting_for_data {
            if self.file_drop_emitted {
                return None;
            }
            let modifiers = self.drop_modifiers.take().unwrap_or(fallback_modifiers);
            return Some(self.drop_now(cursor, modifiers));
        }
        Some(self.hover(cursor, fallback_modifiers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_state_tracks_buttons_cancel_and_screen_ratio() {
        let mut state = PointerState::default();
        state.set_cursor(100.0, 25.0);
        state.set_modifiers(InputModifiers::empty().with_shift());
        assert_eq!(state.set_button_pressed(PRIMARY_MOUSE_BUTTON, true), 1);
        assert_eq!(held_mouse_button(state.buttons), Some(PRIMARY_MOUSE_BUTTON));

        let canceled = state.cancel_mouse(Some(ScreenSpace {
            origin: (400.0, 50.0),
            client_ratio: 2.0,
        }));
        assert_eq!(state.buttons, 0);
        assert_eq!(canceled.phase, PointerPhase::Cancel);
        assert_eq!(canceled.buttons, 1);
        assert_eq!((canceled.screen_x, canceled.screen_y), (600.0, 100.0));
        assert!(canceled.modifiers.shift);
    }

    #[test]
    fn pen_frame_patch_corrects_pen_pressure_and_cancellation_only() {
        let state = PointerState {
            buttons: 1,
            ..PointerState::default()
        };
        let mut pen = InputPayload::Pointer(state.pointer_event(
            PointerDescriptor::simple(1000, PointerType::Pen, true, None),
            PointerPhase::Up,
            0,
            1,
            false,
            InputModifiers::default(),
            None,
            None,
        ));
        PenFramePatch {
            pressure: Some(0.0),
            canceled: true,
        }
        .apply_to(&mut pen);
        let InputPayload::Pointer(pen) = pen else {
            unreachable!();
        };
        assert_eq!(pen.phase, PointerPhase::Cancel);
        assert_eq!(pen.pressure, 0.0);

        let mut mouse = InputPayload::Pointer(PointerInput::mouse(PointerPhase::Up, 1.0, 1.0));
        PenFramePatch {
            pressure: Some(0.0),
            canceled: true,
        }
        .apply_to(&mut mouse);
        let InputPayload::Pointer(mouse) = mouse else {
            unreachable!();
        };
        assert_eq!(mouse.phase, PointerPhase::Up);
    }

    #[test]
    fn drag_transfer_tracker_stages_delayed_drop_and_abandon() {
        let mut drag = DragTransferTracker::default();
        drag.begin(7u64, None);
        assert_eq!(drag.abandon_drop(7), None);

        let held = InputModifiers::from_flags(true, false, false, false);
        drag.wait_for_drop_data(7, 11u64, held);
        assert!(drag.accepts_serial(7, 11));
        assert!(!drag.accepts_serial(7, 12));

        let dropped = drag
            .ingest_paths(
                7,
                vec![PathBuf::from("/tmp/a.txt")],
                (8.0, 16.0),
                InputModifiers::default(),
            )
            .expect("drop event");
        assert_eq!(dropped.kind, FileDragKind::Drop);
        assert_eq!(dropped.modifiers, held);
        assert_eq!(dropped.position, Some((8.0, 16.0)));
        assert_eq!(
            drag.on_dropped((8.0, 16.0), InputModifiers::default()),
            None
        );
    }
}
