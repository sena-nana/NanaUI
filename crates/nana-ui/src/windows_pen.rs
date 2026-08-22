use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows_sys::Win32::UI::Input::Pointer::{
    GetPointerPenInfo, POINTER_FLAG_CANCELED, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_PRIMARY, POINTER_PEN_INFO,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageExtraInfo, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE,
    PEN_MASK_ROTATION, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERUP,
    WM_POINTERUPDATE, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

const SUBCLASS_ID: usize = 0x4E_41_4E_41;
const MAX_PENDING_EVENTS: usize = 512;
const MI_WP_SIGNATURE: usize = 0xFF51_5700;
const SIGNATURE_MASK: usize = 0xFFFF_FF00;
const TOUCH_INDICATOR: usize = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PenPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PenEvent {
    pub phase: PenPhase,
    pub pointer_id: u64,
    pub client_x: i32,
    pub client_y: i32,
    pub screen_x: i32,
    pub screen_y: i32,
    pub button: i16,
    pub buttons: u16,
    pub pressure: f32,
    pub tilt_x: i16,
    pub tilt_y: i16,
    pub twist: u16,
    pub is_primary: bool,
}

struct HookState {
    input: Mutex<HookInputState>,
}

#[derive(Default)]
struct HookInputState {
    events: VecDeque<PenEvent>,
    active: HashMap<u32, PenEvent>,
}

pub(crate) struct WindowsPenHook {
    hwnd: HWND,
    state: *mut HookState,
}

impl WindowsPenHook {
    pub(crate) fn install(window: &winit::window::Window) -> Result<Self, String> {
        let handle = window
            .window_handle()
            .map_err(|error| format!("failed to acquire Win32 window handle: {error}"))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("hosted Windows pen input requires an HWND".into());
        };
        let hwnd = handle.hwnd.get() as *mut c_void;
        let state = Box::into_raw(Box::new(HookState {
            input: Mutex::new(HookInputState::default()),
        }));
        // SAFETY: hwnd belongs to the live winit window. `state` remains allocated
        // until the subclass is removed by `Drop`.
        let installed = unsafe {
            SetWindowSubclass(hwnd, Some(pen_subclass_proc), SUBCLASS_ID, state as usize)
        };
        if installed == 0 {
            // SAFETY: ownership was not transferred because installation failed.
            unsafe { drop(Box::from_raw(state)) };
            return Err("SetWindowSubclass failed for Windows pen input".into());
        }
        Ok(Self { hwnd, state })
    }

    pub(crate) fn drain(&self) -> Vec<PenEvent> {
        // SAFETY: the hook owns `state` for its full lifetime.
        let state = unsafe { &*self.state };
        state
            .input
            .lock()
            .map(|mut input| input.events.drain(..).collect())
            .unwrap_or_default()
    }
}

impl Drop for WindowsPenHook {
    fn drop(&mut self) {
        // SAFETY: the hook is dropped before its winit Window owner. Successful
        // removal guarantees the callback can no longer observe `state`.
        let removed =
            unsafe { RemoveWindowSubclass(self.hwnd, Some(pen_subclass_proc), SUBCLASS_ID) };
        if removed != 0 {
            unsafe { drop(Box::from_raw(self.state)) };
        }
    }
}

unsafe extern "system" fn pen_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> LRESULT {
    if is_promoted_pen_mouse_message(message) {
        // SAFETY: reads metadata for the message currently being dispatched.
        let extra = unsafe { GetMessageExtraInfo() } as usize;
        if is_promoted_pen_extra_info(extra) {
            return 0;
        }
    }

    if matches!(
        message,
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERCAPTURECHANGED
    ) {
        let pointer_id = (wparam as u32) & 0xFFFF;
        let mut info = POINTER_PEN_INFO::default();
        // SAFETY: ref_data is the live HookState installed with this subclass.
        let state = unsafe { &*(ref_data as *const HookState) };
        // SAFETY: `info` is a valid writable POINTER_PEN_INFO.
        let event = if unsafe { GetPointerPenInfo(pointer_id, &mut info) } != 0 {
            let mut client = info.pointerInfo.ptPixelLocation;
            // SAFETY: hwnd is the subclassed live window and `client` is writable.
            let _ = unsafe { ScreenToClient(hwnd, &mut client) };
            Some(pen_event_from_info(message, info, client))
        } else if message == WM_POINTERCAPTURECHANGED {
            state
                .input
                .lock()
                .ok()
                .and_then(|input| input.active.get(&pointer_id).copied().map(cancel_event))
        } else {
            None
        };

        if let Some(event) = event {
            if let Ok(mut input) = state.input.lock() {
                if matches!(event.phase, PenPhase::Up | PenPhase::Cancel) {
                    input.active.remove(&pointer_id);
                } else {
                    input.active.insert(pointer_id, event);
                }
                if input.events.len() == MAX_PENDING_EVENTS {
                    input.events.pop_front();
                }
                input.events.push_back(event);
            }
            // Trigger WM_PAINT so winit wakes and drains the bounded queue even
            // when it does not translate WM_POINTER into a WindowEvent.
            unsafe { InvalidateRect(hwnd, ptr::null(), 0) };
        }
    }

    // SAFETY: forwards all messages not intentionally filtered above to winit's
    // original window procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn cancel_event(mut event: PenEvent) -> PenEvent {
    event.phase = PenPhase::Cancel;
    event.button = -1;
    event.buttons = 0;
    event.pressure = 0.0;
    event
}

fn is_promoted_pen_mouse_message(message: u32) -> bool {
    matches!(
        message,
        WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK
    )
}

fn is_promoted_pen_extra_info(extra: usize) -> bool {
    extra & SIGNATURE_MASK == MI_WP_SIGNATURE && extra & TOUCH_INDICATOR == 0
}

fn pen_event_from_info(message: u32, info: POINTER_PEN_INFO, client: POINT) -> PenEvent {
    let pointer = info.pointerInfo;
    let phase = if message == WM_POINTERDOWN {
        PenPhase::Down
    } else if message == WM_POINTERUP {
        PenPhase::Up
    } else if message == WM_POINTERCAPTURECHANGED
        || pointer.pointerFlags & POINTER_FLAG_CANCELED != 0
    {
        PenPhase::Cancel
    } else {
        PenPhase::Move
    };
    let contact = pointer.pointerFlags & POINTER_FLAG_INCONTACT != 0;
    let barrel = info.penFlags & PEN_FLAG_BARREL != 0;
    let eraser = info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0;
    let mut buttons = 0_u16;
    if eraser {
        buttons |= 32;
    } else if contact || pointer.pointerFlags & POINTER_FLAG_FIRSTBUTTON != 0 {
        buttons |= 1;
    }
    if barrel {
        buttons |= 2;
    }
    if matches!(phase, PenPhase::Up | PenPhase::Cancel) {
        buttons = 0;
    }
    let button = match phase {
        PenPhase::Down | PenPhase::Up if eraser => 5,
        PenPhase::Down | PenPhase::Up if barrel => 2,
        PenPhase::Down | PenPhase::Up => 0,
        PenPhase::Move | PenPhase::Cancel => -1,
    };
    let pressure = if matches!(phase, PenPhase::Up | PenPhase::Cancel) {
        0.0
    } else if info.penMask & PEN_MASK_PRESSURE != 0 {
        (info.pressure as f32 / 1024.0).clamp(0.0, 1.0)
    } else if contact {
        0.5
    } else {
        0.0
    };
    PenEvent {
        phase,
        pointer_id: 0x1_0000 + u64::from(pointer.pointerId),
        client_x: client.x,
        client_y: client.y,
        screen_x: pointer.ptPixelLocation.x,
        screen_y: pointer.ptPixelLocation.y,
        button,
        buttons,
        pressure,
        tilt_x: if info.penMask & PEN_MASK_TILT_X != 0 {
            info.tiltX.clamp(-90, 90) as i16
        } else {
            0
        },
        tilt_y: if info.penMask & PEN_MASK_TILT_Y != 0 {
            info.tiltY.clamp(-90, 90) as i16
        } else {
            0
        },
        twist: if info.penMask & PEN_MASK_ROTATION != 0 {
            (info.rotation % 360) as u16
        } else {
            0
        },
        is_primary: pointer.pointerFlags & POINTER_FLAG_PRIMARY != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_win32_pen_axes_buttons_and_primary_identity() {
        let mut info = POINTER_PEN_INFO::default();
        info.pointerInfo.pointerId = 37;
        info.pointerInfo.pointerFlags =
            POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON | POINTER_FLAG_PRIMARY;
        info.pointerInfo.ptPixelLocation = POINT { x: 900, y: 700 };
        info.penFlags = PEN_FLAG_BARREL;
        info.penMask = PEN_MASK_PRESSURE | PEN_MASK_ROTATION | PEN_MASK_TILT_X | PEN_MASK_TILT_Y;
        info.pressure = 512;
        info.rotation = 450;
        info.tiltX = 120;
        info.tiltY = -120;

        let event = pen_event_from_info(WM_POINTERDOWN, info, POINT { x: 40, y: 50 });
        assert_eq!(event.phase, PenPhase::Down);
        assert_eq!(event.pointer_id, 0x1_0000 + 37);
        assert_eq!((event.client_x, event.client_y), (40, 50));
        assert_eq!((event.screen_x, event.screen_y), (900, 700));
        assert_eq!(event.button, 2);
        assert_eq!(event.buttons, 3);
        assert!((event.pressure - 0.5).abs() < f32::EPSILON);
        assert_eq!((event.tilt_x, event.tilt_y, event.twist), (90, -90, 90));
        assert!(event.is_primary);

        let released = pen_event_from_info(WM_POINTERUP, info, POINT::default());
        assert_eq!(released.buttons, 0);
        assert_eq!(released.pressure, 0.0);

        info.penFlags = PEN_FLAG_ERASER;
        let eraser = pen_event_from_info(WM_POINTERDOWN, info, POINT::default());
        assert_eq!(eraser.button, 5);
        assert_eq!(eraser.buttons, 32);
    }

    #[test]
    fn promoted_pen_signature_excludes_touch_promotions() {
        assert!(is_promoted_pen_extra_info(MI_WP_SIGNATURE));
        assert!(!is_promoted_pen_extra_info(
            MI_WP_SIGNATURE | TOUCH_INDICATOR
        ));
        assert!(!is_promoted_pen_extra_info(0));
    }

    #[test]
    fn cached_pen_state_can_finish_as_pointer_cancel() {
        let event = PenEvent {
            phase: PenPhase::Move,
            pointer_id: 0x1_0025,
            client_x: 40,
            client_y: 50,
            screen_x: 900,
            screen_y: 700,
            button: -1,
            buttons: 3,
            pressure: 0.5,
            tilt_x: 10,
            tilt_y: -10,
            twist: 90,
            is_primary: true,
        };

        let cancelled = cancel_event(event);
        assert_eq!(cancelled.phase, PenPhase::Cancel);
        assert_eq!(cancelled.pointer_id, event.pointer_id);
        assert_eq!(cancelled.button, -1);
        assert_eq!(cancelled.buttons, 0);
        assert_eq!(cancelled.pressure, 0.0);
        assert_eq!((cancelled.client_x, cancelled.client_y), (40, 50));
    }
}
