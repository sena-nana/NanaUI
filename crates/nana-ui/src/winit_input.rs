//! Winit-to-canonical input lowering for NanaUI's desktop host.
//!
//! Pure tracking, button masks, pen pressure/cancel fixes, and OS file-drag
//! staging live in [`nana_ui_input`]; this module owns only the translation
//! from `winit` event types into those backend-neutral structures.

use std::borrow::Cow;
use std::io;
use std::time::Duration;

pub(crate) use nana_ui_input::{
    DragTransferTracker, PRIMARY_MOUSE_BUTTON, PenFramePatch, PointerDescriptor, PointerState,
    ScreenSpace, held_mouse_button, shift_wheel_axes,
};
#[cfg(test)]
pub(crate) use nana_ui_input::{mouse_button_mask, screen_position};
use nana_ui_platform::{
    CompositionInput, DeviceId, FileDragInput, InputModifiers, InputPayload, KeyInput, KeyState,
    LogicalKey, PhysicalKey, PointerId, PointerInput, PointerPhase, PointerType, SystemAppearance,
    WheelInput, WheelUnit, WindowEvent, WindowGeometry, WindowId,
};
use winit::data_transfer::{DataTransferId, TypeHint};
use winit::dpi::PhysicalPosition;
use winit::event::{
    ButtonSource, DeviceId as WinitDeviceId, ElementState, MouseButton, MouseScrollDelta,
    PointerKind, PointerSource, TabletToolData, TabletToolKind, WindowEvent as WinitWindowEvent,
};
use winit::event_loop::{ActiveEventLoop, AsyncRequestSerial, DndAction};
use winit::keyboard::{Key, ModifiersState, PhysicalKey as WinitPhysicalKey};
use winit::window::Theme as WinitTheme;

/// Actions a file drop accepts. AppKit narrows a Control-drag from Finder to
/// Link and refuses the drop when nothing matches, so macOS takes Link too;
/// elsewhere Link would only turn Shift-drags into shortcuts.
#[cfg(target_os = "macos")]
const FILE_DROP_ACTIONS: &[DndAction] = &[DndAction::Copy, DndAction::Link];
#[cfg(not(target_os = "macos"))]
const FILE_DROP_ACTIONS: &[DndAction] = &[DndAction::Copy];

/// What a pointer event does to a running host-driven window move.
#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameMoveStep {
    /// Follow the pointer; the gesture continues.
    Follow,
    /// The gesture is over; drop the session.
    Finish,
    /// Swallow the event; the gesture continues.
    Hold,
}

/// Decides a running window move from one pointer event, given the `owner`
/// button whose release ends it.
#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
pub(crate) fn frame_move_step(phase: PointerPhase, button: i16, owner: i16) -> FrameMoveStep {
    match phase {
        PointerPhase::Move => FrameMoveStep::Follow,
        // The pinned winit win32 proc synthesizes `PointerLeft` from a
        // client-rect bounds check even while the gesture holds capture, so a
        // drag crossing the window edge arrives here as `Cancel`. Ending on it
        // would drop the window the moment it leaves its own former bounds.
        PointerPhase::Cancel => FrameMoveStep::Hold,
        PointerPhase::Up if button == owner => FrameMoveStep::Finish,
        // A fresh primary press stands in for a release the platform never
        // delivered, and matches the system move loop, which any click ends.
        PointerPhase::Down if button == PRIMARY_MOUSE_BUTTON => FrameMoveStep::Finish,
        _ => FrameMoveStep::Hold,
    }
}

/// One native event lowered to canonical input, not yet stamped: the device
/// it came from, its payload, the text a key press typed, and when.
#[derive(Debug)]
pub(crate) struct LoweredInput {
    pub device: DeviceId,
    pub payload: InputPayload,
    /// For a key press: the text it typed, delivered right after the key.
    pub text: Option<String>,
    /// Runtime clock time of the event.
    pub now: Duration,
}

impl LoweredInput {
    pub(crate) fn event(device: DeviceId, payload: InputPayload, now: Duration) -> Self {
        Self {
            device,
            payload,
            text: None,
            now,
        }
    }
}

/// Per-window adapter pairing backend-neutral [`PointerState`] and
/// [`DragTransferTracker`] with `winit` event lowering.
#[derive(Debug, Default)]
pub(crate) struct WinitInputAdapter {
    pub pointer: PointerState,
    pub drag: DragTransferTracker<DataTransferId, AsyncRequestSerial>,
}

impl WinitInputAdapter {
    pub(crate) fn cursor(&self) -> (f32, f32) {
        self.pointer.cursor
    }

    pub(crate) fn set_cursor(&mut self, point: (f32, f32)) {
        self.pointer.set_cursor(point.0, point.1);
    }

    pub(crate) fn buttons(&self) -> u16 {
        self.pointer.buttons
    }

    pub(crate) fn last_device(&self) -> DeviceId {
        self.pointer.last_device
    }

    pub(crate) fn set_modifiers(&mut self, state: ModifiersState) {
        self.pointer.set_modifiers(platform_input_modifiers(state));
    }

    pub(crate) fn clear_pointers(&mut self) {
        self.pointer.clear_buttons();
    }

    pub(crate) fn cancel_mouse(&mut self, screen_origin: Option<ScreenSpace>) -> PointerInput {
        self.pointer.cancel_mouse(screen_origin)
    }

    pub(crate) fn pointer_event(
        &self,
        descriptor: PointerDescriptor,
        phase: PointerPhase,
        button: i16,
        buttons: u16,
        activation_click: bool,
        screen_origin: Option<ScreenSpace>,
        pressure: Option<f32>,
    ) -> PointerInput {
        self.pointer.pointer_event(
            descriptor,
            phase,
            button,
            buttons,
            activation_click,
            self.pointer.modifiers,
            screen_origin,
            pressure,
        )
    }

    pub(crate) fn set_cursor_physical(&mut self, position: PhysicalPosition<f64>, scale: f32) {
        let point = position.to_logical::<f32>(f64::from(scale));
        self.pointer.set_cursor(point.x, point.y);
    }

    /// Keys held during an OS drag. The target window gets no modifier
    /// events while the source owns the keyboard, so prefer the system state
    /// and fall back to the last tracked one where it cannot be sampled.
    fn drag_modifiers(&self) -> InputModifiers {
        system_keyboard_modifiers()
            .map(system_input_modifiers)
            .unwrap_or(self.pointer.modifiers)
    }

    /// Lower one native event to canonical input, or `None` for events that
    /// are not input.
    pub(crate) fn map(
        &mut self,
        event: &WinitWindowEvent,
        scale: f32,
        screen_origin: Option<ScreenSpace>,
        now: Duration,
    ) -> Option<LoweredInput> {
        let modifiers = self.pointer.modifiers;
        let (device, payload, text) = match event {
            WinitWindowEvent::PointerMoved {
                device_id,
                position,
                primary,
                source,
            } => {
                self.set_cursor_physical(*position, scale);
                (
                    device_id,
                    InputPayload::Pointer(self.pointer.pointer_event(
                        map_pointer_source(source, *primary, *device_id),
                        PointerPhase::Move,
                        -1,
                        self.pointer.buttons,
                        false,
                        modifiers,
                        screen_origin,
                        None,
                    )),
                    None,
                )
            }
            WinitWindowEvent::PointerEntered {
                device_id,
                position,
                primary,
                kind,
            } => {
                self.set_cursor_physical(*position, scale);
                (
                    device_id,
                    InputPayload::Pointer(self.pointer.pointer_event(
                        map_pointer_kind(kind, *primary, *device_id),
                        PointerPhase::Move,
                        -1,
                        self.pointer.buttons,
                        false,
                        modifiers,
                        screen_origin,
                        None,
                    )),
                    None,
                )
            }
            WinitWindowEvent::PointerButton {
                device_id,
                state,
                position,
                primary,
                button,
                is_macos_activation_click,
            } => {
                self.set_cursor_physical(*position, scale);
                let mouse = button.clone().mouse_button().unwrap_or(MouseButton::Left);
                let button_code = mouse_button_code(mouse);
                let pressed = *state == ElementState::Pressed;
                let buttons = self.pointer.set_button_pressed(button_code, pressed);
                (
                    device_id,
                    InputPayload::Pointer(self.pointer.pointer_event(
                        map_button_source(button, *primary, *device_id),
                        if pressed {
                            PointerPhase::Down
                        } else {
                            PointerPhase::Up
                        },
                        button_code,
                        buttons,
                        *is_macos_activation_click,
                        modifiers,
                        screen_origin,
                        None,
                    )),
                    None,
                )
            }
            // Leaving the surface is not a cancellation: hover clears while
            // a capture, if any, keeps its pointer. The leave carries its own
            // device, not the last event's.
            WinitWindowEvent::PointerLeft {
                device_id,
                position,
                primary,
                kind,
            } => {
                if let Some(position) = position {
                    self.set_cursor_physical(*position, scale);
                }
                let pointer = map_pointer_kind(kind, *primary, *device_id);
                (
                    device_id,
                    InputPayload::PointerLeave {
                        pointer_id: PointerId(pointer.pointer_id),
                    },
                    None,
                )
            }
            WinitWindowEvent::MouseWheel {
                device_id, delta, ..
            } => {
                let (delta_x, delta_y, unit) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y, WheelUnit::Lines),
                    MouseScrollDelta::PixelDelta(delta) => (
                        (delta.x / f64::from(scale)) as f32,
                        (delta.y / f64::from(scale)) as f32,
                        WheelUnit::Pixels,
                    ),
                    _ => (0.0, 0.0, WheelUnit::Pixels),
                };
                let (delta_x, delta_y) = shift_wheel_axes(
                    (delta_x, delta_y),
                    modifiers.shift,
                    cfg!(target_os = "macos"),
                );
                // The wheel belongs to the mouse: same pointer, same hover.
                (
                    device_id,
                    InputPayload::Wheel(WheelInput {
                        pointer_id: PointerId(1),
                        x: self.pointer.cursor.0,
                        y: self.pointer.cursor.1,
                        delta_x,
                        delta_y,
                        unit,
                        modifiers,
                    }),
                    None,
                )
            }
            // A press synthesized on focus gain was typed into another window,
            // e.g. the Esc that dismissed an owned native dialog; delivering it
            // would run that shortcut a second time here.
            WinitWindowEvent::KeyboardInput {
                event,
                is_synthetic: true,
                ..
            } if event.state == ElementState::Pressed => return None,
            WinitWindowEvent::KeyboardInput {
                device_id, event, ..
            } => {
                let pressed = event.state == ElementState::Pressed;
                let key = KeyInput {
                    physical: PhysicalKey(platform_physical_key(&event.physical_key)),
                    logical: LogicalKey(platform_input_key(&event.logical_key)),
                    state: if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    repeat: event.repeat,
                    modifiers,
                };
                let text = event
                    .text
                    .as_ref()
                    .filter(|text| pressed && !text.is_empty())
                    .map(ToString::to_string);
                (device_id, InputPayload::Key(key), text)
            }
            WinitWindowEvent::Ime(ime) => {
                return Some(LoweredInput::event(
                    self.pointer.last_device,
                    InputPayload::Composition(platform_composition(ime)),
                    now,
                ));
            }
            _ => return None,
        };
        let device = canonical_device_id(*device);
        self.pointer.last_device = device;
        Some(LoweredInput {
            device,
            payload,
            text,
            now,
        })
    }

    pub(crate) fn handle_file_dnd(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        scale: f32,
        event: &WinitWindowEvent,
    ) -> Option<FileDragInput> {
        match event {
            WinitWindowEvent::DragEntered {
                id: transfer,
                position,
            } => {
                if let Some(position) = position {
                    self.set_cursor_physical(*position, scale);
                }
                if !dnd_advertises_files(event_loop, *transfer) {
                    return None;
                }
                let _ = event_loop.set_valid_dnd_actions(*transfer, FILE_DROP_ACTIONS);
                let serial = event_loop
                    .fetch_data_transfer(*transfer, &TypeHint::UriList)
                    .ok();
                self.drag.begin(*transfer, serial);
                self.map_file_window_event(event)
            }
            WinitWindowEvent::DragPosition { position, .. } => {
                self.set_cursor_physical(*position, scale);
                self.map_file_window_event(event)
            }
            WinitWindowEvent::DragDropped { id: transfer, .. } => {
                if self.drag.has_pending_paths() {
                    return self.map_file_window_event(event);
                }
                match event_loop.fetch_data_transfer(*transfer, &TypeHint::UriList) {
                    Ok(serial) => {
                        let modifiers = self.drag_modifiers();
                        self.drag.wait_for_drop_data(*transfer, serial, modifiers);
                        None
                    }
                    Err(_) => self.map_file_window_event(event),
                }
            }
            WinitWindowEvent::DragLeft { .. } => self.map_file_window_event(event),
            WinitWindowEvent::DataTransferReceived {
                id: transfer,
                serial,
                value,
            } => {
                if !self.drag.accepts_serial(*transfer, *serial) {
                    return None;
                }
                match value.try_as_file_paths() {
                    Ok(paths) => {
                        let modifiers = self.drag_modifiers();
                        self.drag
                            .ingest_paths(*transfer, paths, self.pointer.cursor, modifiers)
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Deadlock
                        ) =>
                    {
                        None
                    }
                    // A release waiting on these paths would otherwise never end.
                    Err(_) => self.drag.abandon_drop(*transfer),
                }
            }
            _ => None,
        }
    }

    pub(crate) fn map_file_window_event(
        &mut self,
        event: &WinitWindowEvent,
    ) -> Option<FileDragInput> {
        let cursor = self.pointer.cursor;
        let modifiers = self.drag_modifiers();
        match event {
            WinitWindowEvent::DragEntered { id: transfer, .. } => {
                Some(self.drag.on_entered(*transfer, cursor, modifiers))
            }
            WinitWindowEvent::DragPosition { id: transfer, .. } => {
                self.drag.on_position(*transfer, cursor, modifiers)
            }
            WinitWindowEvent::DragLeft { .. } => Some(self.drag.on_left()),
            WinitWindowEvent::DragDropped { .. } => self.drag.on_dropped(cursor, modifiers),
            _ => None,
        }
    }
}

/// Apply what the platform reported for the pen frame being delivered (see
/// [`nana_window::PenInput`]) to the event the backend lowered from it.
pub(crate) fn apply_pen_frame(payload: &mut InputPayload, frame: Option<nana_window::PenFrame>) {
    if let Some(frame) = frame {
        PenFramePatch {
            pressure: frame.pressure,
            canceled: frame.canceled,
        }
        .apply_to(payload);
    }
}

/// A leave the backend reports for a pen whose leaves the host takes from
/// [`nana_window::PenInput`] instead: winit's Win32 backend sends one after
/// every lift while the pen still hovers.
pub(crate) fn is_backend_pen_leave(event: &WinitWindowEvent, pen: &nana_window::PenInput) -> bool {
    pen.reports_leave()
        && matches!(
            event,
            WinitWindowEvent::PointerLeft {
                kind: PointerKind::TabletTool(_),
                ..
            }
        )
}

pub(crate) fn mouse_button_code(button: MouseButton) -> i16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        other => other as u8 as i16,
    }
}

pub(crate) fn tablet_pointer_id(device_id: Option<WinitDeviceId>, kind: TabletToolKind) -> u64 {
    let device = device_id
        .map(|id| id.into_raw().unsigned_abs())
        .unwrap_or(0);
    let kind_index = u64::from(kind != TabletToolKind::Pen);
    1000 + device.saturating_mul(2) + kind_index
}

/// The pointer id of a pen whose events carry no device, as Win32 pens do.
pub(crate) fn pen_pointer_id() -> u64 {
    tablet_pointer_id(None, TabletToolKind::Pen)
}

fn canonical_device_id(device_id: Option<WinitDeviceId>) -> DeviceId {
    DeviceId(
        device_id
            .map(|id| id.into_raw().unsigned_abs())
            .unwrap_or(0),
    )
}

fn map_tablet(
    kind: TabletToolKind,
    data: &TabletToolData,
    primary: bool,
    device_id: Option<WinitDeviceId>,
) -> PointerDescriptor {
    let tilt = data.clone().tilt();
    let angle = data.clone().angle();
    PointerDescriptor {
        pointer_id: tablet_pointer_id(device_id, kind),
        pointer_type: PointerType::Pen,
        is_primary: primary,
        pressure: data
            .force
            .as_ref()
            .map(|force| force.normalized(angle) as f32),
        tangential_pressure: data.tangential_force.unwrap_or(0.0),
        tilt_x: tilt.map(|tilt| i16::from(tilt.x)).unwrap_or(0),
        tilt_y: tilt.map(|tilt| i16::from(tilt.y)).unwrap_or(0),
        twist: data.twist.unwrap_or(0),
    }
}

fn map_pointer_kind(
    kind: &PointerKind,
    primary: bool,
    device_id: Option<WinitDeviceId>,
) -> PointerDescriptor {
    match kind {
        PointerKind::Touch(finger_id) => PointerDescriptor::simple(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            None,
        ),
        PointerKind::TabletTool(kind) => PointerDescriptor::simple(
            tablet_pointer_id(device_id, *kind),
            PointerType::Pen,
            primary,
            None,
        ),
        PointerKind::Mouse | PointerKind::Unknown | _ => {
            PointerDescriptor::simple(1, PointerType::Mouse, primary, None)
        }
    }
}

fn map_pointer_source(
    source: &PointerSource,
    primary: bool,
    device_id: Option<WinitDeviceId>,
) -> PointerDescriptor {
    match source {
        PointerSource::Touch { finger_id, force } => PointerDescriptor::simple(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            force.as_ref().map(|force| force.normalized(None) as f32),
        ),
        PointerSource::TabletTool { kind, data } => map_tablet(*kind, data, primary, device_id),
        PointerSource::Mouse | PointerSource::Unknown | _ => {
            map_pointer_kind(&PointerKind::Mouse, primary, device_id)
        }
    }
}

fn map_button_source(
    source: &ButtonSource,
    primary: bool,
    device_id: Option<WinitDeviceId>,
) -> PointerDescriptor {
    match source {
        ButtonSource::Touch { finger_id, force } => PointerDescriptor::simple(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            force.as_ref().map(|force| force.normalized(None) as f32),
        ),
        ButtonSource::TabletTool { kind, data, .. } => map_tablet(*kind, data, primary, device_id),
        ButtonSource::Mouse(_) | ButtonSource::Unknown(_) | _ => {
            map_pointer_kind(&PointerKind::Mouse, primary, device_id)
        }
    }
}

pub(crate) fn platform_composition(ime: &winit::event::Ime) -> CompositionInput {
    match ime {
        winit::event::Ime::Enabled => CompositionInput::Enabled,
        winit::event::Ime::Disabled => CompositionInput::Disabled,
        winit::event::Ime::Preedit(text, _) if text.is_empty() => CompositionInput::End,
        winit::event::Ime::Preedit(text, selection) => CompositionInput::Update {
            text: text.clone(),
            selection: *selection,
        },
        winit::event::Ime::Commit(text) => CompositionInput::Commit(text.clone()),
        winit::event::Ime::DeleteSurrounding {
            before_bytes,
            after_bytes,
        } => CompositionInput::DeleteSurrounding {
            before_bytes: *before_bytes,
            after_bytes: *after_bytes,
        },
        _ => CompositionInput::Disabled,
    }
}

pub(crate) fn platform_window_event(
    event: &WinitWindowEvent,
    id: WindowId,
    geometry: WindowGeometry,
) -> Option<WindowEvent> {
    Some(match event {
        WinitWindowEvent::CloseRequested => WindowEvent::CloseRequested { id },
        WinitWindowEvent::Destroyed => WindowEvent::Closed { id },
        WinitWindowEvent::Occluded(hidden) => WindowEvent::VisibilityChanged {
            id,
            hidden: *hidden,
        },
        WinitWindowEvent::Focused(focused) => WindowEvent::FocusChanged {
            id,
            focused: *focused,
        },
        WinitWindowEvent::SurfaceResized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
            WindowEvent::Resized { id, geometry }
        }
        WinitWindowEvent::Moved(_) => WindowEvent::Moved { id, geometry },
        WinitWindowEvent::ThemeChanged(theme) => WindowEvent::AppearanceChanged {
            id,
            appearance: system_appearance_from_winit(*theme),
        },
        _ => return None,
    })
}

/// winit reports light/dark on macOS, Windows, Android and web; the remaining
/// platforms never emit `ThemeChanged`, so no appearance event is synthesised.
pub(crate) const fn system_appearance_from_winit(theme: WinitTheme) -> SystemAppearance {
    match theme {
        WinitTheme::Light => SystemAppearance::Light,
        WinitTheme::Dark => SystemAppearance::Dark,
    }
}

/// The layout-resolved key name: a named key's name (`ArrowLeft`, `Enter`),
/// a character key's character.
pub(crate) fn platform_input_key(key: &Key) -> Cow<'static, str> {
    match key {
        Key::Named(named) => format!("{named:?}").into(),
        Key::Character(character) => character.to_string().into(),
        Key::Dead(_) => "Dead".into(),
        Key::Unidentified(_) => "Unidentified".into(),
    }
}

/// The key's position as a W3C `code` (`KeyA`, `Digit1`, `ShiftLeft`).
pub(crate) fn platform_physical_key(key: &WinitPhysicalKey) -> Cow<'static, str> {
    match key {
        WinitPhysicalKey::Code(code) => format!("{code:?}").into(),
        WinitPhysicalKey::Unidentified(_) => "Unidentified".into(),
    }
}

pub(crate) fn platform_input_modifiers(value: ModifiersState) -> InputModifiers {
    InputModifiers {
        alt: value.alt_key(),
        control: value.control_key(),
        meta: value.meta_key(),
        shift: value.shift_key(),
    }
}

fn system_input_modifiers(keys: nana_window::KeyboardModifiers) -> InputModifiers {
    InputModifiers {
        alt: keys.alt,
        control: keys.control,
        meta: keys.meta,
        shift: keys.shift,
    }
}

#[cfg(not(test))]
fn system_keyboard_modifiers() -> Option<nana_window::KeyboardModifiers> {
    nana_window::keyboard_modifiers()
}

#[cfg(test)]
fn system_keyboard_modifiers() -> Option<nana_window::KeyboardModifiers> {
    tests::SYSTEM_KEYS.with(std::cell::Cell::get)
}

fn dnd_advertises_files(event_loop: &dyn ActiveEventLoop, transfer: DataTransferId) -> bool {
    event_loop
        .data_transfer(transfer)
        .map(|transfer| transfer.has_type(&TypeHint::UriList))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    thread_local! {
        /// The system modifier sample this test thread sees; `None` is a
        /// platform that cannot sample it.
        pub(super) static SYSTEM_KEYS: std::cell::Cell<Option<nana_window::KeyboardModifiers>> =
            const { std::cell::Cell::new(None) };
    }

    use super::*;
    use nana_ui_platform::FileDragKind;
    use winit::event::{FingerId, TouchPhase};
    use winit::keyboard::NamedKey;

    fn map_input(
        adapter: &mut WinitInputAdapter,
        event: &WinitWindowEvent,
        scale: f32,
        origin: Option<ScreenSpace>,
    ) -> Option<InputPayload> {
        adapter
            .map(event, scale, origin, Duration::ZERO)
            .map(|lowered| lowered.payload)
    }

    fn geometry() -> WindowGeometry {
        WindowGeometry {
            physical_size: (200, 100),
            logical_size: (100.0, 50.0),
            scale_factor: 2.0,
            ..WindowGeometry::default()
        }
    }

    #[test]
    fn shift_wheel_scrolls_sideways_once() {
        assert_eq!(shift_wheel_axes((0.0, 3.0), true, false), (3.0, 0.0));
        assert_eq!(shift_wheel_axes((3.0, 0.0), true, true), (3.0, 0.0));
        assert_eq!(shift_wheel_axes((0.0, 3.0), false, false), (0.0, 3.0));
    }

    #[test]
    fn input_key_uses_named_debug_and_character_text() {
        assert_eq!(
            platform_input_key(&Key::Named(NamedKey::ArrowDown)),
            "ArrowDown"
        );
        assert_eq!(platform_input_key(&Key::Character("V".into())), "V");
        assert_eq!(
            platform_input_key(&Key::Unidentified(winit::keyboard::NativeKey::Unidentified)),
            "Unidentified"
        );
        assert_eq!(
            platform_physical_key(&WinitPhysicalKey::Code(winit::keyboard::KeyCode::KeyA)),
            "KeyA"
        );
    }

    #[test]
    fn modifiers_map_control_alt_shift_and_meta() {
        let modifiers = platform_input_modifiers(
            ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SHIFT
                | ModifiersState::META,
        );
        assert!(modifiers.control);
        assert!(modifiers.alt);
        assert!(modifiers.shift);
        assert!(modifiers.meta);
    }

    #[test]
    fn physical_cursor_positions_become_logical() {
        let mut adapter = WinitInputAdapter::default();
        adapter.set_cursor_physical(PhysicalPosition::new(100.0, 50.0), 2.0);
        assert_eq!(adapter.cursor(), (50.0, 25.0));
        adapter.set_cursor_physical(PhysicalPosition::new(220.0, 110.0), 2.0);
        assert_eq!(adapter.cursor(), (110.0, 55.0));
    }

    #[test]
    fn focus_gain_synthetic_press_is_not_delivered_but_real_keys_are() {
        let escape = |state, is_synthetic| WinitWindowEvent::KeyboardInput {
            device_id: None,
            event: winit::event::KeyEvent {
                physical_key: WinitPhysicalKey::Code(winit::keyboard::KeyCode::Escape),
                logical_key: Key::Named(NamedKey::Escape),
                text: None,
                location: winit::keyboard::KeyLocation::Standard,
                state,
                repeat: false,
                text_with_all_modifiers: None,
                key_without_modifiers: Key::Named(NamedKey::Escape),
            },
            is_synthetic,
        };
        let mut adapter = WinitInputAdapter::default();
        assert!(
            map_input(
                &mut adapter,
                &escape(ElementState::Pressed, true),
                1.0,
                None
            )
            .is_none()
        );
        assert!(matches!(
            map_input(
                &mut adapter,
                &escape(ElementState::Released, true),
                1.0,
                None
            ),
            Some(InputPayload::Key(KeyInput {
                state: KeyState::Released,
                ..
            }))
        ));
        assert!(matches!(
            map_input(&mut adapter, &escape(ElementState::Pressed, false), 1.0, None),
            Some(InputPayload::Key(ref key)) if key.is_pressed() && key.logical.0 == "Escape"
        ));
    }

    #[test]
    fn mouse_buttons_use_the_hosted_mask_contract() {
        assert_eq!(mouse_button_code(MouseButton::Left), 0);
        assert_eq!(mouse_button_code(MouseButton::Right), 2);
        assert_eq!(mouse_button_mask(0), 1);
        assert_eq!(mouse_button_mask(1), 4);
        assert_eq!(mouse_button_mask(2), 2);
    }

    #[test]
    fn a_window_move_follows_its_own_button_and_survives_a_synthetic_cancel() {
        const MIDDLE: i16 = 1;
        assert_eq!(
            frame_move_step(PointerPhase::Move, -1, MIDDLE),
            FrameMoveStep::Follow
        );
        assert_eq!(
            frame_move_step(PointerPhase::Up, PRIMARY_MOUSE_BUTTON, MIDDLE),
            FrameMoveStep::Hold
        );
        assert_eq!(
            frame_move_step(PointerPhase::Up, MIDDLE, MIDDLE),
            FrameMoveStep::Finish
        );
        assert_eq!(
            frame_move_step(PointerPhase::Cancel, -1, MIDDLE),
            FrameMoveStep::Hold
        );
        assert_eq!(
            frame_move_step(PointerPhase::Down, PRIMARY_MOUSE_BUTTON, MIDDLE),
            FrameMoveStep::Finish
        );
    }

    #[test]
    fn a_held_gesture_reports_its_primary_button_first() {
        assert_eq!(held_mouse_button(0), None);
        assert_eq!(held_mouse_button(mouse_button_mask(1)), Some(1));
        assert_eq!(held_mouse_button(mouse_button_mask(2)), Some(2));
        assert_eq!(
            held_mouse_button(mouse_button_mask(PRIMARY_MOUSE_BUTTON) | mouse_button_mask(1)),
            Some(PRIMARY_MOUSE_BUTTON)
        );
    }

    #[test]
    fn mouse_cancel_ends_a_press_whose_release_never_arrives() {
        let mut adapter = WinitInputAdapter::default();
        map_input(
            &mut adapter,
            &WinitWindowEvent::PointerButton {
                device_id: None,
                state: ElementState::Pressed,
                position: PhysicalPosition::new(20.0, 10.0),
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Left),
                is_macos_activation_click: false,
            },
            1.0,
            None,
        );
        let PointerInput {
            phase,
            pointer_type,
            buttons,
            ..
        } = adapter.cancel_mouse(None);
        assert_eq!(phase, PointerPhase::Cancel);
        assert_eq!(pointer_type, PointerType::Mouse);
        assert_eq!(buttons, 1);

        let Some(InputPayload::Pointer(PointerInput { phase, buttons, .. })) = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerMoved {
                device_id: None,
                position: PhysicalPosition::new(60.0, 10.0),
                primary: true,
                source: PointerSource::Mouse,
            },
            1.0,
            None,
        ) else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Move);
        assert_eq!(buttons, 0);
    }

    #[test]
    fn pointer_move_down_and_leave_match_hosted_coordinates() {
        let mut adapter = WinitInputAdapter::default();
        let moved = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerMoved {
                device_id: None,
                position: PhysicalPosition::new(20.0, 40.0),
                primary: true,
                source: PointerSource::Mouse,
            },
            2.0,
            Some(ScreenSpace {
                origin: (100.0, 200.0),
                client_ratio: 1.0,
            }),
        )
        .expect("cursor move");
        let InputPayload::Pointer(PointerInput {
            phase,
            x,
            y,
            screen_x,
            screen_y,
            pointer_type,
            button,
            ..
        }) = moved
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Move);
        assert_eq!(pointer_type, PointerType::Mouse);
        assert_eq!((x, y), (10.0, 20.0));
        assert_eq!((screen_x, screen_y), (110.0, 220.0));
        assert_eq!(button, -1);

        let down = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerButton {
                device_id: None,
                state: ElementState::Pressed,
                position: PhysicalPosition::new(20.0, 40.0),
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Left),
                is_macos_activation_click: false,
            },
            2.0,
            Some(ScreenSpace {
                origin: (100.0, 200.0),
                client_ratio: 1.0,
            }),
        )
        .expect("mouse down");
        let InputPayload::Pointer(PointerInput {
            phase,
            buttons,
            pressure,
            ..
        }) = down
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Down);
        assert_eq!(buttons, 1);
        assert_eq!(pressure, 0.5);

        let activation = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerButton {
                device_id: None,
                state: ElementState::Pressed,
                position: PhysicalPosition::new(20.0, 40.0),
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Left),
                is_macos_activation_click: true,
            },
            2.0,
            Some(ScreenSpace {
                origin: (100.0, 200.0),
                client_ratio: 1.0,
            }),
        )
        .expect("activation down");
        let InputPayload::Pointer(PointerInput {
            activation_click, ..
        }) = activation
        else {
            panic!("expected pointer");
        };
        assert!(activation_click);

        let left = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerLeft {
                device_id: None,
                position: None,
                primary: true,
                kind: PointerKind::Mouse,
            },
            2.0,
            Some(ScreenSpace {
                origin: (100.0, 200.0),
                client_ratio: 1.0,
            }),
        )
        .expect("cursor left");
        assert_eq!(
            left,
            InputPayload::PointerLeave {
                pointer_id: PointerId(1)
            }
        );
        assert_eq!(adapter.cursor().0, 10.0);
        assert_eq!(adapter.buttons(), 1);
    }

    #[test]
    fn pointer_enter_emits_move_without_a_followup_moved() {
        let mut adapter = WinitInputAdapter::default();
        let entered = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerEntered {
                device_id: None,
                position: PhysicalPosition::new(20.0, 40.0),
                primary: true,
                kind: PointerKind::Mouse,
            },
            2.0,
            Some(ScreenSpace {
                origin: (100.0, 200.0),
                client_ratio: 1.0,
            }),
        )
        .expect("pointer enter");
        let InputPayload::Pointer(PointerInput {
            phase,
            x,
            y,
            screen_x,
            screen_y,
            pointer_type,
            button,
            ..
        }) = entered
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Move);
        assert_eq!(pointer_type, PointerType::Mouse);
        assert_eq!((x, y), (10.0, 20.0));
        assert_eq!((screen_x, screen_y), (110.0, 220.0));
        assert_eq!(button, -1);
    }

    #[test]
    fn pointer_leave_uses_event_position_when_present() {
        let mut adapter = WinitInputAdapter::default();
        adapter.set_cursor((1.0, 1.0));
        let left = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerLeft {
                device_id: None,
                position: Some(PhysicalPosition::new(40.0, 80.0)),
                primary: true,
                kind: PointerKind::Mouse,
            },
            2.0,
            None,
        )
        .expect("pointer leave");
        assert!(matches!(left, InputPayload::PointerLeave { .. }));
        assert_eq!(adapter.cursor(), (20.0, 40.0));
    }

    #[test]
    fn a_drag_leaving_the_window_keeps_its_buttons() {
        let mut adapter = WinitInputAdapter::default();
        adapter
            .pointer
            .set_button_pressed(PRIMARY_MOUSE_BUTTON, true);
        map_input(
            &mut adapter,
            &WinitWindowEvent::PointerLeft {
                device_id: None,
                position: None,
                primary: true,
                kind: PointerKind::Mouse,
            },
            1.0,
            None,
        )
        .expect("pointer leave");
        assert_eq!(adapter.buttons(), mouse_button_mask(PRIMARY_MOUSE_BUTTON));
    }

    #[test]
    fn tablet_pointer_ids_split_device_and_tool_kind() {
        let pen = WinitDeviceId::from_raw(2);
        let other = WinitDeviceId::from_raw(3);
        assert_ne!(
            tablet_pointer_id(Some(pen), TabletToolKind::Pen),
            tablet_pointer_id(Some(pen), TabletToolKind::Eraser)
        );
        assert_ne!(
            tablet_pointer_id(Some(pen), TabletToolKind::Pen),
            tablet_pointer_id(Some(other), TabletToolKind::Pen)
        );
        let mut adapter = WinitInputAdapter::default();
        let moved = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerMoved {
                device_id: Some(pen),
                position: PhysicalPosition::new(4.0, 8.0),
                primary: true,
                source: PointerSource::TabletTool {
                    kind: TabletToolKind::Eraser,
                    data: TabletToolData::default(),
                },
            },
            1.0,
            None,
        )
        .expect("pen move");
        let InputPayload::Pointer(PointerInput {
            pointer_id,
            pointer_type,
            ..
        }) = moved
        else {
            panic!("expected pointer");
        };
        assert_eq!(pointer_type, PointerType::Pen);
        assert_eq!(
            pointer_id.0,
            tablet_pointer_id(Some(pen), TabletToolKind::Eraser)
        );
        assert_ne!(pointer_id.0, 1000);
    }

    #[test]
    fn wheel_preserves_line_delta_and_converts_pixels() {
        let mut adapter = WinitInputAdapter::default();
        adapter.set_cursor((8.0, 16.0));
        let line = map_input(
            &mut adapter,
            &WinitWindowEvent::MouseWheel {
                device_id: None,
                delta: MouseScrollDelta::LineDelta(1.0, -2.0),
                phase: TouchPhase::Moved,
            },
            2.0,
            None,
        )
        .expect("line wheel");
        assert_eq!(
            line,
            InputPayload::Wheel(WheelInput {
                pointer_id: PointerId(1),
                x: 8.0,
                y: 16.0,
                delta_x: 1.0,
                delta_y: -2.0,
                unit: WheelUnit::Lines,
                modifiers: Default::default(),
            })
        );

        let pixel = map_input(
            &mut adapter,
            &WinitWindowEvent::MouseWheel {
                device_id: None,
                delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(8.0, -4.0)),
                phase: TouchPhase::Moved,
            },
            2.0,
            None,
        )
        .expect("pixel wheel");
        let InputPayload::Wheel(WheelInput {
            delta_x,
            delta_y,
            unit,
            ..
        }) = pixel
        else {
            panic!("expected wheel");
        };
        assert_eq!((delta_x, delta_y), (4.0, -2.0));
        assert_eq!(unit, WheelUnit::Pixels);
    }

    #[test]
    fn first_touch_is_primary_and_uses_hosted_pointer_id() {
        let mut adapter = WinitInputAdapter::default();
        let start = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerButton {
                device_id: None,
                state: ElementState::Pressed,
                position: PhysicalPosition::new(4.0, 8.0),
                primary: true,
                button: ButtonSource::Touch {
                    finger_id: FingerId::from_raw(3),
                    force: None,
                },
                is_macos_activation_click: false,
            },
            2.0,
            None,
        )
        .expect("touch start");
        let InputPayload::Pointer(PointerInput {
            phase,
            pointer_id,
            pointer_type,
            is_primary,
            x,
            y,
            buttons,
            ..
        }) = start
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Down);
        assert_eq!(pointer_id, PointerId(5));
        assert_eq!(pointer_type, PointerType::Touch);
        assert!(is_primary);
        assert_eq!((x, y), (2.0, 4.0));
        assert_eq!(buttons, 1);
    }

    #[test]
    fn ime_and_window_lifecycle_mapping_is_backend_neutral() {
        assert_eq!(
            platform_composition(&winit::event::Ime::Commit("你".into())),
            CompositionInput::Commit("你".into())
        );
        assert_eq!(
            platform_composition(&winit::event::Ime::Preedit("かな".into(), Some((3, 6)))),
            CompositionInput::Update {
                text: "かな".into(),
                selection: Some((3, 6)),
            }
        );
        assert_eq!(
            platform_composition(&winit::event::Ime::DeleteSurrounding {
                before_bytes: 3,
                after_bytes: 0,
            }),
            CompositionInput::DeleteSurrounding {
                before_bytes: 3,
                after_bytes: 0,
            }
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::CloseRequested,
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::CloseRequested {
                id: WindowId::PRIMARY
            })
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::Focused(true),
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::FocusChanged {
                id: WindowId::PRIMARY,
                focused: true,
            })
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::Occluded(true),
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::VisibilityChanged {
                id: WindowId::PRIMARY,
                hidden: true,
            })
        );
        assert!(
            platform_window_event(
                &WinitWindowEvent::Ime(winit::event::Ime::Enabled),
                WindowId::PRIMARY,
                geometry(),
            )
            .is_none()
        );
        assert!(
            platform_window_event(
                &WinitWindowEvent::RedrawRequested,
                WindowId::PRIMARY,
                geometry(),
            )
            .is_none()
        );
    }

    #[test]
    fn file_drag_batches_hover_paths_and_emits_one_drop() {
        let mut adapter = WinitInputAdapter::default();
        adapter.set_cursor((24.0, 48.0));
        let transfer = DataTransferId::from_raw(1);
        assert!(matches!(
            adapter.map_file_window_event(&WinitWindowEvent::DragEntered {
                id: transfer,
                position: None,
            }),
            Some(FileDragInput {
                kind: FileDragKind::Hover,
                ..
            })
        ));
        assert!(matches!(
            adapter.map_file_window_event(&WinitWindowEvent::DragDropped {
                id: transfer,
                proposed_action: None,
            }),
            Some(FileDragInput {
                kind: FileDragKind::Drop,
                ..
            })
        ));
        assert!(
            adapter
                .map_file_window_event(&WinitWindowEvent::DragDropped {
                    id: transfer,
                    proposed_action: None,
                })
                .is_none()
        );

        let mut cancelled = WinitInputAdapter::default();
        cancelled.map_file_window_event(&WinitWindowEvent::DragEntered {
            id: transfer,
            position: None,
        });
        assert!(matches!(
            cancelled.map_file_window_event(&WinitWindowEvent::DragLeft { id: transfer }),
            Some(FileDragInput {
                kind: FileDragKind::Cancel,
                ..
            })
        ));
    }

    #[test]
    fn an_unreadable_release_ends_the_drag() {
        let transfer = DataTransferId::from_raw(5);
        let mut adapter = WinitInputAdapter::default();
        adapter.drag.begin(transfer, None);
        assert!(adapter.drag.abandon_drop(transfer).is_none());
        adapter.drag.wait_for_drop_data(
            transfer,
            AsyncRequestSerial::get(),
            InputModifiers::default(),
        );
        assert_eq!(
            adapter.drag.abandon_drop(transfer),
            Some(FileDragInput::cancel())
        );
        assert!(adapter.drag.abandon_drop(transfer).is_none());
    }

    #[test]
    fn file_drag_events_carry_the_held_modifiers() {
        for (sample, expected) in [(None, true), (Some(Default::default()), false)] {
            SYSTEM_KEYS.with(|keys| keys.set(sample));
            let transfer = DataTransferId::from_raw(3);
            let mut adapter = WinitInputAdapter::default();
            adapter.set_modifiers(ModifiersState::CONTROL);
            let hovered = adapter.map_file_window_event(&WinitWindowEvent::DragEntered {
                id: transfer,
                position: None,
            });
            assert!(matches!(
                hovered,
                Some(FileDragInput { kind: FileDragKind::Hover, modifiers, .. }) if modifiers.control == expected
            ));
            let dropped = adapter.map_file_window_event(&WinitWindowEvent::DragDropped {
                id: transfer,
                proposed_action: None,
            });
            assert!(matches!(
                dropped,
                Some(FileDragInput { kind: FileDragKind::Drop, modifiers, .. }) if modifiers.control == expected
            ));
        }
    }

    #[test]
    fn file_drag_ingests_fetched_paths_before_and_after_drop() {
        let transfer = DataTransferId::from_raw(7);
        let paths = vec![std::path::PathBuf::from("/tmp/nana.txt")];
        let mut hover = WinitInputAdapter::default();
        hover.set_cursor((8.0, 16.0));
        hover.drag.begin(transfer, None);
        assert_eq!(
            hover.drag.ingest_paths(
                transfer,
                paths.clone(),
                hover.cursor(),
                InputModifiers::default()
            ),
            Some(FileDragInput {
                kind: FileDragKind::Hover,
                paths: paths.clone(),
                position: Some((8.0, 16.0)),
                modifiers: InputModifiers::default(),
            })
        );
        assert_eq!(
            hover.map_file_window_event(&WinitWindowEvent::DragDropped {
                id: transfer,
                proposed_action: None,
            }),
            Some(FileDragInput {
                kind: FileDragKind::Drop,
                paths: paths.clone(),
                position: Some((8.0, 16.0)),
                modifiers: InputModifiers::default(),
            })
        );

        let mut delayed = WinitInputAdapter::default();
        SYSTEM_KEYS.with(|keys| {
            keys.set(Some(nana_window::KeyboardModifiers {
                control: true,
                ..Default::default()
            }))
        });
        let held = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        delayed
            .drag
            .wait_for_drop_data(transfer, AsyncRequestSerial::get(), held);
        SYSTEM_KEYS.with(|keys| keys.set(Some(Default::default())));
        assert_eq!(
            delayed.drag.ingest_paths(
                transfer,
                paths.clone(),
                delayed.cursor(),
                InputModifiers::default()
            ),
            Some(FileDragInput {
                kind: FileDragKind::Drop,
                paths,
                position: Some((0.0, 0.0)),
                modifiers: held,
            })
        );
        assert!(
            delayed
                .map_file_window_event(&WinitWindowEvent::DragDropped {
                    id: transfer,
                    proposed_action: None,
                })
                .is_none()
        );
    }

    #[test]
    fn screen_position_falls_back_to_client_without_origin() {
        assert_eq!(screen_position(None, (3.0, 4.0)), (3.0, 4.0));
        assert_eq!(
            screen_position(
                Some(ScreenSpace {
                    origin: (10.0, 20.0),
                    client_ratio: 1.0,
                }),
                (3.0, 4.0)
            ),
            (13.0, 24.0)
        );
    }

    #[test]
    fn a_client_point_is_converted_before_it_is_added_to_the_origin() {
        let space = ScreenSpace {
            origin: (400.0, 50.0),
            client_ratio: 2.0,
        };
        assert_eq!(screen_position(Some(space), (100.0, 25.0)), (600.0, 100.0));
        let space = ScreenSpace {
            origin: (400.0, 50.0),
            client_ratio: 0.5,
        };
        assert_eq!(screen_position(Some(space), (100.0, 24.0)), (450.0, 62.0));
    }

    #[test]
    fn pen_frame_facts_correct_pressure_and_cancellation() {
        let mut adapter = WinitInputAdapter::default();
        let pen_button = |state| WinitWindowEvent::PointerButton {
            device_id: None,
            state,
            position: PhysicalPosition::new(20.0, 40.0),
            primary: true,
            button: ButtonSource::TabletTool {
                kind: TabletToolKind::Pen,
                button: winit::event::TabletToolButton::Contact,
                data: TabletToolData::default(),
            },
            is_macos_activation_click: false,
        };
        let zero = nana_window::PenFrame {
            pressure: Some(0.0),
            canceled: false,
        };

        let mut down =
            map_input(&mut adapter, &pen_button(ElementState::Pressed), 1.0, None).expect("down");
        apply_pen_frame(&mut down, None);
        let InputPayload::Pointer(PointerInput { pressure, .. }) = down else {
            panic!("expected pointer");
        };
        assert_eq!(pressure, 0.5);

        let mut down =
            map_input(&mut adapter, &pen_button(ElementState::Pressed), 1.0, None).expect("down");
        apply_pen_frame(&mut down, Some(zero));
        let InputPayload::Pointer(PointerInput {
            phase, pressure, ..
        }) = down
        else {
            panic!("expected pointer");
        };
        assert_eq!((phase, pressure), (PointerPhase::Down, 0.0));

        let mut up =
            map_input(&mut adapter, &pen_button(ElementState::Released), 1.0, None).expect("up");
        apply_pen_frame(
            &mut up,
            Some(nana_window::PenFrame {
                pressure: Some(0.0),
                canceled: true,
            }),
        );
        let InputPayload::Pointer(PointerInput {
            phase,
            buttons,
            pointer_type,
            ..
        }) = up
        else {
            panic!("expected pointer");
        };
        assert_eq!(pointer_type, PointerType::Pen);
        assert_eq!((phase, buttons), (PointerPhase::Cancel, 0));

        let mut mouse = map_input(
            &mut adapter,
            &WinitWindowEvent::PointerButton {
                device_id: None,
                state: ElementState::Pressed,
                position: PhysicalPosition::new(1.0, 1.0),
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Left),
                is_macos_activation_click: false,
            },
            1.0,
            None,
        )
        .expect("mouse down");
        apply_pen_frame(&mut mouse, Some(zero));
        let InputPayload::Pointer(PointerInput { pressure, .. }) = mouse else {
            panic!("expected pointer");
        };
        assert_eq!(pressure, 0.5);
    }
}
