//! Unified per-HWND Win32 subclass dispatcher.
//!
//! Previously, 8+ features (`chrome`, `size_move`, `work_area`, `pen`,
//! `auxiliary`, `underlay`, `platform::windows` menu/taskbar, `shadow`,
//! `splash`) each installed separate `SetWindowSubclass` chains with raw
//! `Box::into_raw` allocations and manual `WM_NCDESTROY` cleanup.
//!
//! This module installs at most ONE `SetWindowSubclass` per `HWND` (with
//! `Rc`-managed state and deterministic `WM_NCDESTROY` teardown) and routes
//! messages to typed feature slots.

use std::cell::{Cell, RefCell};
use std::ptr;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetWindowLongPtrW, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
    HTTRANSPARENT, HWND_TOPMOST, KillTimer, MINMAXINFO, PostMessageW, SPI_SETWORKAREA, STYLESTRUCT,
    SW_HIDE, SWP_NOZORDER, SetCursor, SetTimer, ShowWindow, WINDOWPLACEMENT, WINDOWPOS, WM_CLOSE,
    WM_COMMAND, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
    WM_GETMINMAXINFO, WM_MOUSEACTIVATE, WM_MOVE, WM_NCDESTROY, WM_NCHITTEST, WM_POINTERDOWN,
    WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, WM_SETCURSOR, WM_SETTINGCHANGE, WM_SIZE,
    WM_SIZING, WM_STYLECHANGING, WM_TIMER, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

use crate::aspect::{AspectLock, SizingBasis};
use crate::pen::PenFrame;
use crate::win32::WindowProp;

const NANA_WINDOW_SUBCLASS_ID: usize = 0x4E_41_57_48; // "NAWH"
const SIZE_MOVE_WAKE_TIMER: usize = 0x4E_41_53_4D; // "NASM"
const SIZE_MOVE_WAKE_INTERVAL_MS: u32 = 16;

static HOOKS_PROP: WindowProp = WindowProp::new("NanaUI.WindowHooks");

#[derive(Clone, Copy)]
pub(crate) struct TaskbarHookState {
    pub had_button: bool,
    pub app_window: bool,
    pub owner: HWND,
}

pub(crate) struct AuxiliaryLevelGuard {
    pub expected: Cell<Option<bool>>,
    pub attached: Cell<bool>,
}

pub(crate) struct SizeMoveHookState {
    pub active: AtomicBool,
    pub wake: Option<Arc<dyn Fn() + Send + Sync>>,
    pub aspect: Cell<Option<AspectLock>>,
    pub basis: Cell<Option<SizingBasis>>,
}

pub(crate) struct PenHookState {
    pub frame: Cell<Option<PenFrame>>,
    pub left: Cell<bool>,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// Callback interface implemented by `underlay`, `shadow`, and `splash`
/// placement records attached to a `Win32WindowHooks`.
pub(crate) trait UnderlayHookTarget {
    fn parent_hwnd(&self) -> HWND;
    fn sync(&self);
}

pub(crate) trait ShadowHookTarget {
    fn companion_hwnd(&self) -> HWND;
    fn sync(&self);
}

pub(crate) trait SplashHookTarget {
    fn sync_from_owner(&self);
    fn layout_splash(&self);
}

pub(crate) struct Win32WindowHooks {
    pub hwnd: HWND,
    destroyed: Cell<bool>,
    pub style_mask: Cell<isize>,
    pub menu_enabled: Cell<bool>,
    pub taskbar: Cell<Option<TaskbarHookState>>,
    pub owned_style_guard: Cell<bool>,
    pub auxiliary_levels: RefCell<Vec<Weak<AuxiliaryLevelGuard>>>,
    pub size_move: RefCell<Option<Rc<SizeMoveHookState>>>,
    pub observe_settings: Cell<bool>,
    pub work_area_placement: Cell<Option<WINDOWPLACEMENT>>,
    pub pen: RefCell<Option<Rc<PenHookState>>>,
    pub underlay_self: RefCell<Option<Weak<dyn UnderlayHookTarget>>>,
    pub underlay_children: RefCell<Vec<Weak<dyn UnderlayHookTarget>>>,
    pub shadow_companions: RefCell<Vec<Weak<dyn ShadowHookTarget>>>,
    pub splash_self: RefCell<Option<Weak<dyn SplashHookTarget>>>,
    pub splash_owner: RefCell<Option<Weak<dyn SplashHookTarget>>>,
    pub custom_cursor: RefCell<Option<Arc<crate::cursor::OwnedHcursor>>>,
    pub interaction: RefCell<Option<crate::interaction::Win32InteractionHookState>>,
}

impl Win32WindowHooks {
    fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            destroyed: Cell::new(false),
            style_mask: Cell::new(0),
            menu_enabled: Cell::new(false),
            taskbar: Cell::new(None),
            owned_style_guard: Cell::new(false),
            auxiliary_levels: RefCell::new(Vec::new()),
            size_move: RefCell::new(None),
            observe_settings: Cell::new(false),
            work_area_placement: Cell::new(None),
            pen: RefCell::new(None),
            underlay_self: RefCell::new(None),
            underlay_children: RefCell::new(Vec::new()),
            shadow_companions: RefCell::new(Vec::new()),
            splash_self: RefCell::new(None),
            splash_owner: RefCell::new(None),
            custom_cursor: RefCell::new(None),
            interaction: RefCell::new(None),
        }
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.destroyed.get()
    }

    /// Look up existing hooks on `hwnd` without installing one.
    pub(crate) fn get(hwnd: HWND) -> Option<Rc<Self>> {
        let ptr = HOOKS_PROP.get(hwnd) as *const Self;
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `HOOKS_PROP` holds a live `Rc<Win32WindowHooks>` until
        // `WM_NCDESTROY` removes the property and drops the subclass ref.
        unsafe {
            Rc::increment_strong_count(ptr);
            Some(Rc::from_raw(ptr))
        }
    }

    /// Return the existing hooks for `hwnd`, or install the unified subclass.
    pub(crate) fn install_or_get(hwnd: HWND) -> Result<Rc<Self>, String> {
        if let Some(existing) = Self::get(hwnd) {
            return Ok(existing);
        }
        let hooks = Rc::new(Self::new(hwnd));
        let raw = Rc::into_raw(Rc::clone(&hooks));
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(window_hooks_proc),
                NANA_WINDOW_SUBCLASS_ID,
                raw as usize,
            )
        } == 0
        {
            // SAFETY: ownership was not transferred because install failed.
            unsafe { drop(Rc::from_raw(raw)) };
            return Err("SetWindowSubclass failed for Win32WindowHooks".into());
        }
        if !HOOKS_PROP.set(hwnd, raw as usize) {
            unsafe {
                RemoveWindowSubclass(hwnd, Some(window_hooks_proc), NANA_WINDOW_SUBCLASS_ID);
                drop(Rc::from_raw(raw));
            }
            return Err("SetPropW failed for Win32WindowHooks".into());
        }
        Ok(hooks)
    }

    pub(crate) fn clear_size_move(&self) {
        if !self.destroyed.get() {
            unsafe { KillTimer(self.hwnd, SIZE_MOVE_WAKE_TIMER) };
        }
        *self.size_move.borrow_mut() = None;
    }

    pub(crate) fn clear_pen(&self) {
        *self.pen.borrow_mut() = None;
    }

    pub(crate) fn prune_and_collect_underlays(&self) -> Vec<Rc<dyn UnderlayHookTarget>> {
        let mut children = self.underlay_children.borrow_mut();
        let mut live = Vec::with_capacity(children.len());
        children.retain(|weak| {
            if let Some(rc) = weak.upgrade() {
                live.push(rc);
                true
            } else {
                false
            }
        });
        live
    }

    pub(crate) fn prune_and_collect_shadows(&self) -> Vec<Rc<dyn ShadowHookTarget>> {
        let mut companions = self.shadow_companions.borrow_mut();
        let mut live = Vec::with_capacity(companions.len());
        companions.retain(|weak| {
            if let Some(rc) = weak.upgrade() {
                live.push(rc);
                true
            } else {
                false
            }
        });
        live
    }

    fn auxiliary_wants_topmost(&self) -> bool {
        let mut levels = self.auxiliary_levels.borrow_mut();
        let mut wants = false;
        levels.retain(|weak| {
            if let Some(guard) = weak.upgrade() {
                if guard.attached.get() && guard.expected.get() == Some(true) {
                    wants = true;
                }
                guard.attached.get()
            } else {
                false
            }
        });
        wants
    }
}

unsafe extern "system" fn window_hooks_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    ref_data: usize,
) -> LRESULT {
    let ptr = ref_data as *const Win32WindowHooks;
    // SAFETY: `ref_data` is the `Rc::into_raw` retained by the subclass until
    // `WM_NCDESTROY`. Hold a local `Rc` across `DefSubclassProc` so reentrant
    // destruction cannot free `Win32WindowHooks` underneath us.
    let hooks = unsafe {
        Rc::increment_strong_count(ptr);
        Rc::from_raw(ptr)
    };

    if message == WM_NCDESTROY {
        HOOKS_PROP.remove(hwnd);
        hooks.destroyed.set(true);
        // SAFETY: detach before forwarding, matching the documented Win32 order.
        unsafe { RemoveWindowSubclass(hwnd, Some(window_hooks_proc), id) };

        if hooks.size_move.borrow().is_some() {
            unsafe { KillTimer(hwnd, SIZE_MOVE_WAKE_TIMER) };
        }
        *hooks.size_move.borrow_mut() = None;
        *hooks.pen.borrow_mut() = None;
        hooks.work_area_placement.set(None);
        *hooks.splash_self.borrow_mut() = None;
        *hooks.splash_owner.borrow_mut() = None;
        *hooks.underlay_self.borrow_mut() = None;
        *hooks.custom_cursor.borrow_mut() = None;

        for shadow in hooks.prune_and_collect_shadows() {
            unsafe { ShowWindow(shadow.companion_hwnd(), SW_HIDE) };
        }
        hooks.shadow_companions.borrow_mut().clear();
        hooks.underlay_children.borrow_mut().clear();
        hooks.auxiliary_levels.borrow_mut().clear();

        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if let Some(taskbar) = hooks.taskbar.take()
            && !taskbar.owner.is_null()
        {
            // Closed once this window is gone: destroying an owner reaches
            // the windows it owns, this one included while it is still being
            // destroyed. `DefWindowProcW` destroys it on `WM_CLOSE`.
            unsafe { PostMessageW(taskbar.owner, WM_CLOSE, 0, 0) };
        }
        // Drop the subclass's own reference now that it is removed.
        unsafe { drop(Rc::from_raw(ptr)) };
        return result;
    }

    // Pre-forward hooks (hit-testing / activation / underlay constraints / menu commands).
    let underlay_self = hooks
        .underlay_self
        .borrow()
        .as_ref()
        .and_then(Weak::upgrade);
    if let Some(underlay) = &underlay_self {
        match message {
            WM_NCHITTEST => {
                return windows_sys::Win32::UI::WindowsAndMessaging::HTTRANSPARENT as LRESULT;
            }
            WM_MOUSEACTIVATE => {
                return windows_sys::Win32::UI::WindowsAndMessaging::MA_NOACTIVATE as LRESULT;
            }
            WM_WINDOWPOSCHANGING if lparam != 0 => {
                let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                if pos.flags & SWP_NOZORDER == 0 {
                    pos.hwndInsertAfter = underlay.parent_hwnd();
                }
            }
            WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE && lparam != 0 => {
                let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
                style.styleNew =
                    (style.styleNew | WS_EX_NOACTIVATE) & !(WS_EX_APPWINDOW | WS_EX_TOOLWINDOW);
            }
            _ => {}
        }
    }

    if message == WM_NCHITTEST
        && hooks
            .splash_self
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some()
    {
        return HTTRANSPARENT as LRESULT;
    }

    if message == WM_NCHITTEST {
        let state = hooks.interaction.borrow().clone();
        if let Some(state) = state
            && let Some(hit) = win32_nchittest(hwnd, lparam, &state)
        {
            return hit;
        }
    }

    if message == WM_SETCURSOR && (lparam as u32 & 0xffff) == HTCLIENT {
        let custom = hooks.custom_cursor.borrow().clone();
        if let Some(custom) = custom {
            unsafe { SetCursor(custom.raw()) };
            return 1;
        }
    }

    if hooks.menu_enabled.get()
        && message == WM_COMMAND
        && lparam == 0
        && (wparam >> 16) & 0xffff == 0
    {
        let cmd_id = (wparam & 0xffff) as u32;
        if cmd_id != 0 {
            crate::menu::push_activation(cmd_id);
            return 0;
        }
    }

    let pen_state = hooks.pen.borrow().clone();
    if let Some(pen) = &pen_state
        && message == WM_POINTERLEAVE
        && crate::pen::windows_is_pen(wparam)
        && !pen.left.replace(true)
    {
        (pen.wake)();
    }

    // Forward to the rest of the subclass chain / winit WndProc.
    // For pen frames, keep the frame active only during `DefSubclassProc`.
    let result = if let Some(pen) = &pen_state
        && matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP)
        && let Some(frame) = crate::pen::windows_pen_frame(wparam)
    {
        let outer = pen.frame.replace(Some(frame));
        let res = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        pen.frame.set(outer);
        res
    } else {
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    };

    // Post-forward hooks.
    if message == WM_SETTINGCHANGE && hooks.observe_settings.get() {
        crate::motion_preference::observe_setting_change(wparam);
        crate::contrast_preference::observe_setting_change(wparam);
    }

    let size_move = hooks.size_move.borrow().clone();
    if let Some(sm) = &size_move {
        if matches!(message, WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE) {
            let active = crate::size_move::size_move_active_after(
                message,
                sm.active.load(Ordering::Acquire),
            );
            sm.active.store(active, Ordering::Release);
            let basis = if active && sm.aspect.get().is_some() {
                crate::size_move::measure_basis(hwnd)
            } else {
                None
            };
            sm.basis.set(basis);
            if let Some(wake) = &sm.wake {
                unsafe {
                    if active {
                        SetTimer(hwnd, SIZE_MOVE_WAKE_TIMER, SIZE_MOVE_WAKE_INTERVAL_MS, None);
                    } else {
                        KillTimer(hwnd, SIZE_MOVE_WAKE_TIMER);
                    }
                }
                wake();
            }
            unsafe { InvalidateRect(hwnd, ptr::null(), 0) };
        } else if message == WM_TIMER && wparam == SIZE_MOVE_WAKE_TIMER {
            if sm.active.load(Ordering::Acquire)
                && let Some(wake) = &sm.wake
            {
                wake();
            }
            return 0;
        } else if message == WM_SIZING
            && crate::size_move::constrain_sizing(hwnd, sm, wparam, lparam)
        {
            return 1;
        }
    }

    if message == WM_STYLECHANGING && lparam != 0 {
        if wparam == GWL_STYLE as usize {
            let mask = hooks.style_mask.get();
            if mask != 0 {
                let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
                style.styleNew = crate::chrome::guarded_style(style.styleNew as isize, mask) as u32;
            }
        } else if wparam == GWL_EXSTYLE as usize
            && (hooks.owned_style_guard.get() || hooks.taskbar.get().is_some())
        {
            let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
            style.styleNew &= !WS_EX_APPWINDOW;
        }
    }

    if message == WM_GETMINMAXINFO && hooks.style_mask.get() != 0 {
        unsafe { crate::work_area::fill_maximized(hwnd, lparam as *mut MINMAXINFO) };
    }

    if message == WM_WINDOWPOSCHANGING
        && lparam != 0
        && hooks.auxiliary_wants_topmost()
        && unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST == 0
    {
        let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
        if pos.flags & SWP_NOZORDER == 0 {
            pos.hwndInsertAfter = HWND_TOPMOST;
        }
    }

    if matches!(message, WM_WINDOWPOSCHANGED | WM_SIZE) {
        for underlay in hooks.prune_and_collect_underlays() {
            underlay.sync();
        }
        for shadow in hooks.prune_and_collect_shadows() {
            shadow.sync();
        }
    }

    if matches!(message, WM_MOVE | WM_SIZE | WM_DPICHANGED)
        && let Some(splash) = hooks.splash_owner.borrow().as_ref().and_then(Weak::upgrade)
    {
        splash.sync_from_owner();
    }
    if matches!(message, WM_SIZE | WM_DPICHANGED)
        && let Some(splash) = hooks.splash_self.borrow().as_ref().and_then(Weak::upgrade)
    {
        splash.layout_splash();
    }

    if matches!(message, WM_SETTINGCHANGE | WM_DISPLAYCHANGE)
        && (message != WM_SETTINGCHANGE || wparam as u32 == SPI_SETWORKAREA)
    {
        crate::display_topology::mark_changed();
        if hooks.work_area_placement.get().is_some() {
            crate::work_area::refit_active(hwnd);
        }
    } else if message == WM_DPICHANGED && hooks.work_area_placement.get().is_some() {
        crate::work_area::refit_active(hwnd);
    }

    result
}

fn win32_nchittest(
    hwnd: HWND,
    lparam: LPARAM,
    state: &crate::interaction::Win32InteractionHookState,
) -> Option<LRESULT> {
    if state.mode() == nana_ui_platform::MousePassthroughMode::Passthrough {
        nana_diagnostics::metric!(
            nana_diagnostics::framework::window::INTERACTION_HIT_TEST_PASSTHROUGH
        );
        return Some(HTTRANSPARENT as LRESULT);
    }
    let snapshot = state.snapshot()?;
    let sx = (lparam as u32 & 0xffff) as i16 as i32;
    let sy = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
    let mut pt = POINT { x: sx, y: sy };
    let mut client_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        if ScreenToClient(hwnd, &mut pt) == 0 || GetClientRect(hwnd, &mut client_rect) == 0 {
            return None;
        }
    }
    if pt.x < client_rect.left
        || pt.y < client_rect.top
        || pt.x >= client_rect.right
        || pt.y >= client_rect.bottom
    {
        return None;
    }
    if state.pointer_captured() {
        nana_diagnostics::metric!(nana_diagnostics::framework::window::INTERACTION_HIT_TEST_CLIENT);
        return Some(HTCLIENT as LRESULT);
    }
    let scale = state.scale_factor();
    let lx = pt.x as f32 / scale;
    let ly = pt.y as f32 / scale;
    let class = snapshot.classify(lx, ly);
    match class {
        nana_ui_platform::NativeHitTestClass::Client => {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::window::INTERACTION_HIT_TEST_CLIENT
            );
            Some(HTCLIENT as LRESULT)
        }
        nana_ui_platform::NativeHitTestClass::Drag => {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::window::INTERACTION_HIT_TEST_DRAG
            );
            Some(HTCAPTION as LRESULT)
        }
        nana_ui_platform::NativeHitTestClass::Resize(edge) => {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::window::INTERACTION_HIT_TEST_RESIZE
            );
            let code = match edge {
                nana_ui_platform::WindowResizeEdge::West => HTLEFT,
                nana_ui_platform::WindowResizeEdge::East => HTRIGHT,
                nana_ui_platform::WindowResizeEdge::North => HTTOP,
                nana_ui_platform::WindowResizeEdge::South => HTBOTTOM,
                nana_ui_platform::WindowResizeEdge::NorthWest => HTTOPLEFT,
                nana_ui_platform::WindowResizeEdge::NorthEast => HTTOPRIGHT,
                nana_ui_platform::WindowResizeEdge::SouthWest => HTBOTTOMLEFT,
                nana_ui_platform::WindowResizeEdge::SouthEast => HTBOTTOMRIGHT,
            };
            Some(code as LRESULT)
        }
        nana_ui_platform::NativeHitTestClass::PassThrough => {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::window::INTERACTION_HIT_TEST_PASSTHROUGH
            );
            // `HTTRANSPARENT` only reaches windows of this thread, and turns the
            // move into `WM_NCMOUSEMOVE`, which the host never sees. In `Forward`
            // the host must see the pointer over a hole to hand the window back
            // to the desktop (`WS_EX_TRANSPARENT`), so the hole stays client.
            if state.mode() == nana_ui_platform::MousePassthroughMode::Forward {
                Some(HTCLIENT as LRESULT)
            } else {
                Some(HTTRANSPARENT as LRESULT)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SPI_SETWORKAREA, SendMessageW, WM_DISPLAYCHANGE,
        WM_SETTINGCHANGE, WS_OVERLAPPEDWINDOW,
    };

    use super::Win32WindowHooks;
    use crate::display_topology::take_display_topology_change;

    #[test]
    fn display_and_work_area_changes_mark_the_display_topology() {
        let _serial = crate::test_serial_lock();
        // SAFETY: STATIC is a system window class; all pointers are valid for this call.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                crate::win32::wide("STATIC").as_ptr(),
                ptr::null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                64,
                64,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        let hooks = Win32WindowHooks::install_or_get(hwnd).expect("hook the fixture window");
        let _ = take_display_topology_change();

        // SAFETY: none of these carry a pointer payload; the window is ours.
        unsafe { SendMessageW(hwnd, WM_DISPLAYCHANGE, 32, (1080 << 16) | 1920) };
        assert!(take_display_topology_change(), "WM_DISPLAYCHANGE");

        unsafe { SendMessageW(hwnd, WM_SETTINGCHANGE, SPI_SETWORKAREA as usize, 0) };
        assert!(take_display_topology_change(), "SPI_SETWORKAREA");

        // Other setting changes leave the topology alone.
        unsafe { SendMessageW(hwnd, WM_SETTINGCHANGE, 0x0057, 0) };
        assert!(!take_display_topology_change(), "unrelated setting");

        drop(hooks);
        // SAFETY: the fixture's own window, on its creating thread.
        unsafe { DestroyWindow(hwnd) };
    }
}
