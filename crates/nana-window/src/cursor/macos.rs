use std::ffi::{c_uchar, c_void};
use std::ptr;
use std::sync::Once;
use std::sync::atomic::{AtomicPtr, Ordering};

use objc2::ffi::{OBJC_ASSOCIATION_RETAIN, objc_getAssociatedObject, objc_setAssociatedObject};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{AllocAnyThread, sel};
use objc2_app_kit::{NSBitmapImageRep, NSCursor, NSDeviceRGBColorSpace, NSImage, NSView};
use objc2_foundation::{NSPoint, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{CursorBitmap, CustomCursorCapabilities, CustomCursorOutcome};

#[derive(Clone)]
pub(super) struct MacosCursorHandle {
    cursor: Retained<NSCursor>,
}

// AppKit cursors created from immutable bitmaps are only used on the window thread.
unsafe impl Send for MacosCursorHandle {}
unsafe impl Sync for MacosCursorHandle {}

pub(super) fn capabilities() -> CustomCursorCapabilities {
    CustomCursorCapabilities {
        supported: true,
        multi_dpi: true,
        max_edge: 256,
    }
}

pub(super) fn create_cursor(bitmap: &CursorBitmap) -> Result<MacosCursorHandle, String> {
    let width = bitmap.width as isize;
    let height = bitmap.height as isize;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut::<*mut c_uchar>(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            width * 4,
            32,
        )
    }
    .ok_or_else(|| String::from("failed to allocate NSBitmapImageRep for custom cursor"))?;

    unsafe {
        let dst = std::slice::from_raw_parts_mut(rep.bitmapData(), bitmap.rgba.len());
        dst.copy_from_slice(&bitmap.rgba);
    }

    let logical_size = NSSize::new(
        f64::from(bitmap.logical_width),
        f64::from(bitmap.logical_height),
    );
    let image = NSImage::initWithSize(NSImage::alloc(), logical_size);
    image.addRepresentation(&rep);

    let hotspot = NSPoint::new(bitmap.logical_hotspot[0], bitmap.logical_hotspot[1]);
    let cursor = NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, hotspot);
    Ok(MacosCursorHandle { cursor })
}

static CUSTOM_CURSOR_ASSOC_KEY: u8 = 0;
static ORIGINAL_RESET_CURSOR_RECTS: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static HOOK_ONCE: Once = Once::new();

unsafe extern "C-unwind" fn hooked_reset_cursor_rects(this: *mut AnyObject, cmd: Sel) {
    let custom_ptr = unsafe {
        objc_getAssociatedObject(this, (&raw const CUSTOM_CURSOR_ASSOC_KEY).cast::<c_void>())
    };
    if !custom_ptr.is_null() {
        let view = unsafe { &*this.cast::<NSView>() };
        let cursor = unsafe { &*custom_ptr.cast::<NSCursor>() };
        view.addCursorRect_cursor(view.bounds(), cursor);
        return;
    }
    let orig = ORIGINAL_RESET_CURSOR_RECTS.load(Ordering::Acquire);
    if !orig.is_null() {
        let orig_fn: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(orig) };
        unsafe { orig_fn(this, cmd) };
    }
}

fn ensure_cursor_rect_hook(view: &NSView) {
    HOOK_ONCE.call_once(|| {
        let cls: &AnyClass = view.class();
        let method = unsafe { objc2::ffi::class_getInstanceMethod(cls, sel!(resetCursorRects)) };
        if !method.is_null() {
            let new_imp: Imp =
                unsafe { std::mem::transmute(hooked_reset_cursor_rects as *const ()) };
            let old_imp = unsafe { objc2::ffi::method_setImplementation(method, new_imp) };
            let old_ptr = old_imp.map_or(ptr::null_mut(), |imp| imp as *const () as *mut c_void);
            ORIGINAL_RESET_CURSOR_RECTS.store(old_ptr, Ordering::Release);
        }
    });
}

fn appkit_view<W: HasWindowHandle + ?Sized>(window: &W) -> Option<&NSView> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: live AppKit view owned by the window on the main thread.
    Some(unsafe { handle.ns_view.cast::<NSView>().as_ref() })
}

pub(super) fn apply_custom_cursor<W: HasWindowHandle + ?Sized>(
    window: &W,
    handle: &MacosCursorHandle,
    scale_milli: u32,
) -> CustomCursorOutcome {
    let Some(view) = appkit_view(window) else {
        return CustomCursorOutcome::Unsupported("missing AppKit view handle".into());
    };
    ensure_cursor_rect_hook(view);
    let view_ptr = (view as *const NSView).cast_mut().cast::<AnyObject>();
    let cursor_ptr = Retained::as_ptr(&handle.cursor)
        .cast_mut()
        .cast::<AnyObject>();
    unsafe {
        objc_setAssociatedObject(
            view_ptr,
            (&raw const CUSTOM_CURSOR_ASSOC_KEY).cast::<c_void>(),
            cursor_ptr,
            OBJC_ASSOCIATION_RETAIN,
        );
    }
    if let Some(ns_window) = view.window() {
        ns_window.invalidateCursorRectsForView(view);
    }
    handle.cursor.set();
    CustomCursorOutcome::Applied { scale_milli }
}

pub(super) fn clear_custom_cursor<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(view) = appkit_view(window) else {
        return;
    };
    let view_ptr = (view as *const NSView).cast_mut().cast::<AnyObject>();
    unsafe {
        objc_setAssociatedObject(
            view_ptr,
            (&raw const CUSTOM_CURSOR_ASSOC_KEY).cast::<c_void>(),
            ptr::null_mut(),
            OBJC_ASSOCIATION_RETAIN,
        );
    }
    if let Some(ns_window) = view.window() {
        ns_window.invalidateCursorRectsForView(view);
    }
}
