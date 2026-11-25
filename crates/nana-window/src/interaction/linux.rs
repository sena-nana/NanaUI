use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::Arc;

use nana_ui_platform::{
    InputRegionOutcome, InputRegionSnapshot, MousePassthroughMode, NativeHitTestClass,
    WindowInteractionCapabilities,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

const XSHAPE_INPUT: c_int = 2;

#[derive(Debug, Default)]
pub(super) struct BackendState {
    xfixes_ready: Option<bool>,
    wayland: Option<WaylandCompositor>,
    pointer_captured: bool,
}

#[derive(Debug)]
struct WaylandCompositor {
    display: NonNull<c_void>,
    compositor: NonNull<c_void>,
}

impl Drop for WaylandCompositor {
    fn drop(&mut self) {
        if let Some(lib) = wayland_sys::client::wayland_client_option() {
            unsafe { (lib.wl_proxy_destroy)(self.compositor.as_ptr().cast()) };
        }
    }
}

impl BackendState {
    pub(super) fn set_pointer_captured<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        captured: bool,
    ) {
        self.pointer_captured = captured;
    }

    pub(super) fn apply<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        window: &W,
        _caps: WindowInteractionCapabilities,
        snapshot: &Arc<InputRegionSnapshot>,
        mode: MousePassthroughMode,
        _scale_factor: f32,
    ) -> InputRegionOutcome {
        let (Ok(win_handle), Ok(disp_handle)) = (window.window_handle(), window.display_handle())
        else {
            return InputRegionOutcome::Unsupported;
        };
        match (win_handle.as_raw(), disp_handle.as_raw()) {
            (RawWindowHandle::Xlib(xwin), RawDisplayHandle::Xlib(xdisp)) => {
                let Some(display) = xdisp.display else {
                    return InputRegionOutcome::Unsupported;
                };
                self.apply_x11(display.as_ptr().cast(), xwin.window, snapshot, mode)
            }
            (RawWindowHandle::Wayland(wl_win), RawDisplayHandle::Wayland(wl_disp)) => {
                self.apply_wayland(wl_disp.display, wl_win.surface, snapshot, mode)
            }
            _ => InputRegionOutcome::Unsupported,
        }
    }

    fn apply_x11(
        &mut self,
        display: *mut x11_dl::xlib::Display,
        window: x11_dl::xlib::Window,
        snapshot: &InputRegionSnapshot,
        mode: MousePassthroughMode,
    ) -> InputRegionOutcome {
        let Ok(xlib) = x11_dl::xlib::Xlib::open() else {
            return InputRegionOutcome::Unsupported;
        };
        let Ok(xfixes) = x11_dl::xfixes::Xlib::open() else {
            return InputRegionOutcome::ForwardFallback {
                rect_count: snapshot.rects.len(),
            };
        };
        let ready = *self.xfixes_ready.get_or_insert_with(|| {
            let mut event_base = 0;
            let mut error_base = 0;
            unsafe { (xfixes.XFixesQueryExtension)(display, &mut event_base, &mut error_base) != 0 }
        });
        if !ready {
            return InputRegionOutcome::ForwardFallback {
                rect_count: snapshot.rects.len(),
            };
        }

        if mode == MousePassthroughMode::Off
            && !self.pointer_captured
            && is_full_window_client(snapshot)
        {
            // Normal opaque single-client window: reset ShapeInput to default window bounds.
            unsafe {
                (xfixes.XFixesSetWindowShapeRegion)(display, window, XSHAPE_INPUT, 0, 0, 0);
                (xlib.XFlush)(display);
            }
            return InputRegionOutcome::WholeWindow { passthrough: false };
        }

        if mode == MousePassthroughMode::Passthrough && !self.pointer_captured {
            let region = unsafe { (xfixes.XFixesCreateRegion)(display, std::ptr::null_mut(), 0) };
            unsafe {
                (xfixes.XFixesSetWindowShapeRegion)(display, window, XSHAPE_INPUT, 0, 0, region);
                (xfixes.XFixesDestroyRegion)(display, region);
                (xlib.XFlush)(display);
            }
            return InputRegionOutcome::WholeWindow { passthrough: true };
        }

        let mut xrects: Vec<x11_dl::xlib::XRectangle> = snapshot
            .physical_input_rects()
            .into_iter()
            .map(|(x, y, w, h)| x11_dl::xlib::XRectangle {
                x: x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                y: y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                width: w.clamp(1, u16::MAX as i32) as u16,
                height: h.clamp(1, u16::MAX as i32) as u16,
            })
            .collect();
        let rect_count = xrects.len();
        unsafe {
            let region = (xfixes.XFixesCreateRegion)(
                display,
                if xrects.is_empty() {
                    std::ptr::null_mut()
                } else {
                    xrects.as_mut_ptr()
                },
                xrects.len() as c_int,
            );
            (xfixes.XFixesSetWindowShapeRegion)(display, window, XSHAPE_INPUT, 0, 0, region);
            (xfixes.XFixesDestroyRegion)(display, region);
            (xlib.XFlush)(display);
        }
        InputRegionOutcome::NativeApplied { rect_count }
    }

    fn apply_wayland(
        &mut self,
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        snapshot: &InputRegionSnapshot,
        mode: MousePassthroughMode,
    ) -> InputRegionOutcome {
        let Some(lib) = wayland_sys::client::wayland_client_option() else {
            return InputRegionOutcome::ForwardFallback {
                rect_count: snapshot.rects.len(),
            };
        };

        if mode == MousePassthroughMode::Off
            && !self.pointer_captured
            && is_full_window_client(snapshot)
        {
            // Opcode 5 on wl_surface = set_input_region(?object). Null clears to full surface.
            let mut arg = wayland_sys::common::wl_argument {
                o: std::ptr::null(),
            };
            unsafe {
                (lib.wl_proxy_marshal_array)(surface.as_ptr().cast(), 5, &mut arg);
                (lib.wl_display_flush)(display.as_ptr().cast());
            }
            return InputRegionOutcome::WholeWindow { passthrough: false };
        }

        if self
            .wayland
            .as_ref()
            .is_none_or(|state| state.display != display)
        {
            self.wayland = bind_wl_compositor(lib, display);
        }
        let Some(wl) = &self.wayland else {
            return InputRegionOutcome::ForwardFallback {
                rect_count: snapshot.rects.len(),
            };
        };

        // `wl_compositor.create_region` (opcode 1) -> `wl_region`.
        let mut create_args = [wayland_sys::common::wl_argument {
            o: std::ptr::null(),
        }];
        let region_proxy = unsafe {
            (lib.wl_proxy_marshal_array_constructor)(
                wl.compositor.as_ptr().cast(),
                1,
                create_args.as_mut_ptr(),
                &WL_REGION_INTERFACE,
            )
        };
        if region_proxy.is_null() {
            return InputRegionOutcome::ForwardFallback {
                rect_count: snapshot.rects.len(),
            };
        }

        let mut rect_count = 0usize;
        if mode != MousePassthroughMode::Passthrough || self.pointer_captured {
            for rect in &snapshot.rects {
                if rect.class == NativeHitTestClass::PassThrough {
                    continue;
                }
                let x = rect.x.floor() as i32;
                let y = rect.y.floor() as i32;
                let w = ((rect.x + rect.width).ceil() as i32 - x).max(1);
                let h = ((rect.y + rect.height).ceil() as i32 - y).max(1);
                // `wl_region.add` (opcode 1): (x, y, width, height)
                let mut add_args = [
                    wayland_sys::common::wl_argument { i: x },
                    wayland_sys::common::wl_argument { i: y },
                    wayland_sys::common::wl_argument { i: w },
                    wayland_sys::common::wl_argument { i: h },
                ];
                unsafe {
                    (lib.wl_proxy_marshal_array)(region_proxy, 1, add_args.as_mut_ptr());
                }
                rect_count += 1;
            }
        }

        // `wl_surface.set_input_region` (opcode 5), then `wl_region.destroy` (opcode 0).
        let mut set_arg = wayland_sys::common::wl_argument {
            o: region_proxy.cast(),
        };
        unsafe {
            (lib.wl_proxy_marshal_array)(surface.as_ptr().cast(), 5, &mut set_arg);
            let mut empty = [];
            (lib.wl_proxy_marshal_array)(region_proxy, 0, empty.as_mut_ptr());
            (lib.wl_proxy_destroy)(region_proxy);
            (lib.wl_display_flush)(display.as_ptr().cast());
        }

        if mode == MousePassthroughMode::Passthrough && !self.pointer_captured {
            InputRegionOutcome::WholeWindow { passthrough: true }
        } else {
            InputRegionOutcome::NativeApplied { rect_count }
        }
    }

    pub(super) fn release(&mut self) {
        self.wayland = None;
    }
}

pub(super) fn probe_capabilities<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    window: &W,
) -> WindowInteractionCapabilities {
    let (Ok(win), Ok(disp)) = (window.window_handle(), window.display_handle()) else {
        return WindowInteractionCapabilities::default();
    };
    match (win.as_raw(), disp.as_raw()) {
        (RawWindowHandle::Xlib(_), RawDisplayHandle::Xlib(xdisp)) if xdisp.display.is_some() => {
            let has_xfixes = x11_dl::xfixes::Xlib::open().is_ok();
            WindowInteractionCapabilities {
                input_region: has_xfixes,
                non_client_chrome: false,
                passthrough_forward: true,
            }
        }
        (RawWindowHandle::Wayland(_), RawDisplayHandle::Wayland(_)) => {
            WindowInteractionCapabilities {
                input_region: wayland_sys::client::wayland_client_option().is_some(),
                non_client_chrome: false,
                passthrough_forward: true,
            }
        }
        _ => WindowInteractionCapabilities::default(),
    }
}

#[repr(transparent)]
struct SyncInterfaces<const N: usize>([*const wayland_sys::common::wl_interface; N]);
// SAFETY: Pointed-to `wl_interface` records are immutable `'static` C descriptors.
unsafe impl<const N: usize> Sync for SyncInterfaces<N> {}

#[repr(transparent)]
struct SyncMessages<const N: usize>([wayland_sys::common::wl_message; N]);
// SAFETY: Pointed-to C strings and interface tables are immutable `'static` data.
unsafe impl<const N: usize> Sync for SyncMessages<N> {}

static WL_REGISTRY_BIND_SIG: &[u8] = b"usun\0";
static WL_REGISTRY_GLOBAL_SIG: &[u8] = b"usu\0";
static WL_REGISTRY_GLOBAL_REMOVE_SIG: &[u8] = b"u\0";
static WL_COMPOSITOR_CREATE_SURFACE_SIG: &[u8] = b"n\0";
static WL_COMPOSITOR_CREATE_REGION_SIG: &[u8] = b"n\0";
static WL_REGION_DESTROY_SIG: &[u8] = b"\0";
static WL_REGION_ADD_SIG: &[u8] = b"iiii\0";

static WL_REGISTRY_NAME: &[u8] = b"wl_registry\0";
static WL_COMPOSITOR_NAME: &[u8] = b"wl_compositor\0";
static WL_REGION_NAME: &[u8] = b"wl_region\0";

static WL_EMPTY_TYPES: SyncInterfaces<4> = SyncInterfaces([
    std::ptr::null(),
    std::ptr::null(),
    std::ptr::null(),
    std::ptr::null(),
]);

static WL_REGISTRY_REQUESTS: SyncMessages<1> = SyncMessages([wayland_sys::common::wl_message {
    name: b"bind\0".as_ptr().cast(),
    signature: WL_REGISTRY_BIND_SIG.as_ptr().cast(),
    types: WL_EMPTY_TYPES.0.as_ptr(),
}]);

static WL_REGISTRY_EVENTS: SyncMessages<2> = SyncMessages([
    wayland_sys::common::wl_message {
        name: b"global\0".as_ptr().cast(),
        signature: WL_REGISTRY_GLOBAL_SIG.as_ptr().cast(),
        types: WL_EMPTY_TYPES.0.as_ptr(),
    },
    wayland_sys::common::wl_message {
        name: b"global_remove\0".as_ptr().cast(),
        signature: WL_REGISTRY_GLOBAL_REMOVE_SIG.as_ptr().cast(),
        types: WL_EMPTY_TYPES.0.as_ptr(),
    },
]);

static WL_REGISTRY_INTERFACE: wayland_sys::common::wl_interface =
    wayland_sys::common::wl_interface {
        name: WL_REGISTRY_NAME.as_ptr().cast(),
        version: 1,
        request_count: 1,
        requests: WL_REGISTRY_REQUESTS.0.as_ptr(),
        event_count: 2,
        events: WL_REGISTRY_EVENTS.0.as_ptr(),
    };

static WL_REGION_REQUESTS: SyncMessages<2> = SyncMessages([
    wayland_sys::common::wl_message {
        name: b"destroy\0".as_ptr().cast(),
        signature: WL_REGION_DESTROY_SIG.as_ptr().cast(),
        types: WL_EMPTY_TYPES.0.as_ptr(),
    },
    wayland_sys::common::wl_message {
        name: b"add\0".as_ptr().cast(),
        signature: WL_REGION_ADD_SIG.as_ptr().cast(),
        types: WL_EMPTY_TYPES.0.as_ptr(),
    },
]);

static WL_REGION_INTERFACE: wayland_sys::common::wl_interface = wayland_sys::common::wl_interface {
    name: WL_REGION_NAME.as_ptr().cast(),
    version: 1,
    request_count: 2,
    requests: WL_REGION_REQUESTS.0.as_ptr(),
    event_count: 0,
    events: std::ptr::null(),
};

static WL_COMPOSITOR_TYPES: SyncInterfaces<2> =
    SyncInterfaces([std::ptr::null(), &WL_REGION_INTERFACE]);

static WL_COMPOSITOR_REQUESTS: SyncMessages<2> = SyncMessages([
    wayland_sys::common::wl_message {
        name: b"create_surface\0".as_ptr().cast(),
        signature: WL_COMPOSITOR_CREATE_SURFACE_SIG.as_ptr().cast(),
        types: WL_EMPTY_TYPES.0.as_ptr(),
    },
    wayland_sys::common::wl_message {
        name: b"create_region\0".as_ptr().cast(),
        signature: WL_COMPOSITOR_CREATE_REGION_SIG.as_ptr().cast(),
        types: WL_COMPOSITOR_TYPES.0.as_ptr().wrapping_add(1),
    },
]);

static WL_COMPOSITOR_INTERFACE: wayland_sys::common::wl_interface =
    wayland_sys::common::wl_interface {
        name: WL_COMPOSITOR_NAME.as_ptr().cast(),
        version: 6,
        request_count: 2,
        requests: WL_COMPOSITOR_REQUESTS.0.as_ptr(),
        event_count: 0,
        events: std::ptr::null(),
    };

fn bind_wl_compositor(
    lib: &'static wayland_sys::client::WaylandClient,
    display: NonNull<c_void>,
) -> Option<WaylandCompositor> {
    struct GlobalHit {
        name: u32,
        version: u32,
    }
    unsafe extern "C" fn on_global(
        data: *mut c_void,
        _registry: *mut c_void,
        name: u32,
        interface: *const c_char,
        version: u32,
    ) {
        if data.is_null() || interface.is_null() {
            return;
        }
        let iface = unsafe { CStr::from_ptr(interface) };
        if iface.to_bytes() == b"wl_compositor" {
            let slot = unsafe { &mut *(data as *mut Option<GlobalHit>) };
            if slot.is_none() {
                *slot = Some(GlobalHit { name, version });
            }
        }
    }
    unsafe extern "C" fn on_global_remove(_data: *mut c_void, _registry: *mut c_void, _name: u32) {}

    let queue = unsafe { (lib.wl_display_create_queue)(display.as_ptr().cast()) };
    if queue.is_null() {
        return None;
    }
    let wrapper = unsafe { (lib.wl_proxy_create_wrapper)(display.as_ptr().cast()) };
    if wrapper.is_null() {
        unsafe { (lib.wl_event_queue_destroy)(queue) };
        return None;
    }
    unsafe { (lib.wl_proxy_set_queue)(wrapper, queue) };

    // `wl_display.get_registry` (opcode 1)
    let mut get_reg_args = [wayland_sys::common::wl_argument {
        o: std::ptr::null(),
    }];
    let registry = unsafe {
        (lib.wl_proxy_marshal_array_constructor)(
            wrapper,
            1,
            get_reg_args.as_mut_ptr(),
            &WL_REGISTRY_INTERFACE,
        )
    };
    unsafe { (lib.wl_proxy_wrapper_destroy)(wrapper) };
    if registry.is_null() {
        unsafe { (lib.wl_event_queue_destroy)(queue) };
        return None;
    }

    let mut found: Option<GlobalHit> = None;
    let mut listener: [extern "C" fn(); 2] = unsafe {
        [
            std::mem::transmute::<
                unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
                extern "C" fn(),
            >(on_global),
            std::mem::transmute::<
                unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
                extern "C" fn(),
            >(on_global_remove),
        ]
    };
    unsafe {
        (lib.wl_proxy_add_listener)(registry, listener.as_mut_ptr(), (&raw mut found).cast());
        (lib.wl_display_roundtrip_queue)(display.as_ptr().cast(), queue);
    }

    let compositor = found.and_then(|hit| {
        let mut bind_args = [
            wayland_sys::common::wl_argument { u: hit.name },
            wayland_sys::common::wl_argument {
                s: WL_COMPOSITOR_NAME.as_ptr().cast(),
            },
            wayland_sys::common::wl_argument {
                u: hit.version.min(6),
            },
            wayland_sys::common::wl_argument {
                o: std::ptr::null(),
            },
        ];
        let proxy = unsafe {
            (lib.wl_proxy_marshal_array_constructor_versioned)(
                registry,
                0,
                bind_args.as_mut_ptr(),
                &WL_COMPOSITOR_INTERFACE,
                hit.version.min(6),
            )
        };
        NonNull::new(proxy.cast())
    });

    unsafe {
        (lib.wl_proxy_destroy)(registry);
        (lib.wl_event_queue_destroy)(queue);
    }
    compositor.map(|compositor| WaylandCompositor {
        display,
        compositor,
    })
}

fn is_full_window_client(snapshot: &InputRegionSnapshot) -> bool {
    if snapshot.rects.len() != 1 {
        return false;
    }
    let rect = &snapshot.rects[0];
    rect.class == NativeHitTestClass::Client
        && snapshot.logical_size.0 > 0.0
        && snapshot.logical_size.1 > 0.0
        && rect.x <= 0.0
        && rect.y <= 0.0
        && rect.x + rect.width >= snapshot.logical_size.0 - 0.5
        && rect.y + rect.height >= snapshot.logical_size.1 - 0.5
}

#[cfg(test)]
mod tests {
    use super::is_full_window_client;
    use nana_ui_platform::{InputRegionRect, InputRegionSnapshot, NativeHitTestClass};

    #[test]
    fn full_window_short_circuit_requires_full_bounds_client_rect() {
        let full_client = InputRegionSnapshot {
            revision: 1,
            rects: vec![InputRegionRect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
                class: NativeHitTestClass::Client,
            }],
            logical_size: (400.0, 300.0),
            scale_factor: 1.0,
            capped: false,
            source_rect_count: 1,
        };
        assert!(is_full_window_client(&full_client));

        // A PassThrough root with one smaller Client pill has rects.len() == 1
        // and must NOT reset OS ShapeInput / wl_region to the whole window.
        let smaller_pill = InputRegionSnapshot {
            rects: vec![InputRegionRect {
                x: 100.0,
                y: 80.0,
                width: 200.0,
                height: 80.0,
                class: NativeHitTestClass::Client,
            }],
            ..full_client.clone()
        };
        assert!(!is_full_window_client(&smaller_pill));

        let full_drag = InputRegionSnapshot {
            rects: vec![InputRegionRect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
                class: NativeHitTestClass::Drag,
            }],
            ..full_client
        };
        assert!(!is_full_window_client(&full_drag));
    }
}
