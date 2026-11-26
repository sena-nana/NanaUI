// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{ActionHandler, ActivationHandler, TreeUpdate};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::*,
        UI::{Accessibility::*, Shell::*, WindowsAndMessaging::*},
    },
    core::*,
};

use crate::{Adapter, QueuedEvents};

fn win32_error() -> ! {
    panic!("{}", Error::from_thread())
}

/// Paired with `subclass_proc`, this uniquely identifies AccessKit's ComCtl32
/// window subclass registration on an `HWND`.
const SUBCLASS_ID: usize = 0x4143_4B54;

struct SubclassState {
    adapter: Adapter,
    activation_handler: Box<dyn ActivationHandler>,
}

struct SubclassImpl {
    hwnd: HWND,
    state: RefCell<SubclassState>,
    installed: Cell<bool>,
    window_destroyed: Cell<bool>,
}

unsafe extern "system" fn subclass_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    ref_data: usize,
) -> LRESULT {
    let ptr = ref_data as *const SubclassImpl;
    if ptr.is_null() {
        return unsafe { DefSubclassProc(window, message, wparam, lparam) };
    }
    // SAFETY: `ref_data` is the `Rc::into_raw` retained by the subclass until
    // it is removed in `WM_NCDESTROY` or `uninstall`. Hold a local `Rc` across
    // message handling and `DefSubclassProc` so reentrant adapter drop or
    // window destruction cannot free `SubclassImpl` while this frame runs.
    let r#impl = unsafe {
        Rc::increment_strong_count(ptr);
        Rc::from_raw(ptr)
    };

    if message == WM_NCDESTROY {
        r#impl.window_destroyed.set(true);
        if r#impl.installed.replace(false) {
            let removed =
                unsafe { RemoveWindowSubclass(window, Some(subclass_proc), id) }.as_bool();
            if removed {
                // SAFETY: Ownership of the subclass's `Rc` reference ends here.
                unsafe { drop(Rc::from_raw(ptr)) };
            }
        }
        r#impl.disconnect_uia();
        return unsafe { DefSubclassProc(window, message, wparam, lparam) };
    }

    if !r#impl.window_destroyed.get() && r#impl.installed.get() {
        match message {
            WM_GETOBJECT => {
                if let Ok(mut state) = r#impl.state.try_borrow_mut() {
                    let state_mut = &mut *state;
                    if let Some(result) = state_mut.adapter.handle_wm_getobject(
                        wparam,
                        lparam,
                        &mut *state_mut.activation_handler,
                    ) {
                        drop(state);
                        return result.into();
                    }
                }
            }
            WM_SETFOCUS | WM_EXITMENULOOP | WM_EXITSIZEMOVE => {
                r#impl.update_window_focus_state(true);
            }
            WM_KILLFOCUS | WM_ENTERMENULOOP | WM_ENTERSIZEMOVE => {
                r#impl.update_window_focus_state(false);
            }
            _ => (),
        }
    }

    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

impl SubclassImpl {
    fn new(
        hwnd: HWND,
        activation_handler: impl 'static + ActivationHandler,
        action_handler: impl 'static + ActionHandler + Send,
    ) -> Rc<Self> {
        let adapter = Adapter::new(hwnd, false, action_handler);
        let state = RefCell::new(SubclassState {
            adapter,
            activation_handler: Box::new(activation_handler),
        });
        Rc::new(Self {
            hwnd,
            state,
            installed: Cell::new(false),
            window_destroyed: Cell::new(false),
        })
    }

    fn install(this: &Rc<Self>) {
        if unsafe { GetWindowSubclass(this.hwnd, Some(subclass_proc), SUBCLASS_ID, None) }.as_bool()
        {
            panic!(
                "subclassing adapter already instantiated on window {:?}",
                this.hwnd.0
            );
        }
        let raw = Rc::into_raw(Rc::clone(this));
        if !unsafe { SetWindowSubclass(this.hwnd, Some(subclass_proc), SUBCLASS_ID, raw as usize) }
            .as_bool()
        {
            // SAFETY: Subclass registration failed; reclaim the leaked reference.
            unsafe { drop(Rc::from_raw(raw)) };
            win32_error();
        }
        this.installed.set(true);
    }

    fn update_window_focus_state(&self, is_focused: bool) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            if let Some(events) = state.adapter.update_window_focus_state(is_focused) {
                drop(state);
                events.raise();
            }
        }
    }

    fn disconnect_uia(&self) {
        let maybe_nodes = match self.state.try_borrow_mut() {
            Ok(mut state) => state.adapter.disconnect_providers(),
            Err(_) => Some(Vec::new()),
        };
        let Some(nodes) = maybe_nodes else {
            return;
        };
        for node in nodes {
            let el: IRawElementProviderSimple = node.into_interface();
            let _ = unsafe { UiaDisconnectProvider(&el) };
        }
        unsafe {
            UiaReturnRawElementProvider(self.hwnd, WPARAM(0), LPARAM(0), None);
        }
    }

    fn uninstall(this: &Rc<Self>) {
        if !this.installed.replace(false) {
            return;
        }
        if !this.window_destroyed.get() {
            let removed =
                unsafe { RemoveWindowSubclass(this.hwnd, Some(subclass_proc), SUBCLASS_ID) }
                    .as_bool();
            if removed {
                // SAFETY: `SetWindowSubclass` retained one strong reference created
                // via `Rc::into_raw`.
                unsafe { Rc::decrement_strong_count(Rc::as_ptr(this)) };
            }
            this.disconnect_uia();
        }
    }
}

/// Uses [Win32 subclassing] to handle `WM_GETOBJECT` messages on a window
/// that provides no other way of adding custom message handlers.
///
/// [Win32 subclassing]: https://docs.microsoft.com/en-us/windows/win32/controls/subclassing-overview
pub struct SubclassingAdapter(Rc<SubclassImpl>);

impl SubclassingAdapter {
    /// Creates a new Windows platform adapter using window subclassing.
    /// This must be done before the window is shown or focused
    /// for the first time.
    ///
    /// This must be called on the thread that owns the window. The activation
    /// handler will always be called on that thread. The action handler
    /// may or may not be called on that thread.
    ///
    /// # Panics
    ///
    /// Panics if the window is already visible.
    pub fn new(
        hwnd: HWND,
        activation_handler: impl 'static + ActivationHandler,
        action_handler: impl 'static + ActionHandler + Send,
    ) -> Self {
        if unsafe { IsWindowVisible(hwnd) }.into() {
            panic!(
                "The AccessKit Windows subclassing adapter must be created before the window is shown (made visible) for the first time."
            );
        }

        let r#impl = SubclassImpl::new(hwnd, activation_handler, action_handler);
        SubclassImpl::install(&r#impl);
        Self(r#impl)
    }

    /// If and only if the tree has been initialized, call the provided function
    /// and apply the resulting update. Note: If the caller's implementation of
    /// [`ActivationHandler::request_initial_tree`] initially returned `None`,
    /// the [`TreeUpdate`] returned by the provided function must contain
    /// a full tree.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it.
    pub fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<QueuedEvents> {
        // SAFETY: We use `RefCell::borrow_mut` here, even though
        // `RefCell::get_mut` is allowed (because this method takes
        // a mutable self reference), just in case there's some way
        // this method can be called from within the subclassed window
        // procedure, e.g. via `ActivationHandler`.
        let mut state = self.0.state.borrow_mut();
        state.adapter.update_if_active(update_factory)
    }
}

impl Drop for SubclassingAdapter {
    fn drop(&mut self) {
        SubclassImpl::uninstall(&self.0);
    }
}
