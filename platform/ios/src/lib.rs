//! Experimental iOS host (`include/nana_ios_host.h`).
//!
//! The Swift shell (`platform/ios/App`) owns the UIKit view, its
//! `CAMetalLayer`, the `CADisplayLink`, the `UITextInput` proxy and the
//! camera layer under the surface; it lowers UIKit events into these calls.
//! The document, input routing, frame demand and the device live in
//! [`nana_mobile_host`], shared with the Android host. Every call is made on
//! the main thread; the callbacks never call back in synchronously.

use std::ffi::c_void;
use std::time::Instant;

use nana_mobile_host::{
    FrameDemand, MobileAppearance, MobilePlatform, MobileRenderer, MobileRuntime, MobileViewport,
    SafeArea, ScrollPhysics, SurfaceOptions, TouchPhase,
};
use nana_mobile_probe::{PreviewUploader, Probe, ProbeState};
use nana_ui_core::LogicalRect;
use nana_ui_platform::{
    CompositionInput, MemoryClipboard, SharedClipboardHost, TextInputContext, shared_clipboard,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NanaSafeArea {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl From<NanaSafeArea> for SafeArea {
    fn from(area: NanaSafeArea) -> Self {
        Self {
            top: area.top,
            right: area.right,
            bottom: area.bottom,
            left: area.left,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NanaRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NanaIosCallbacks {
    pub context: *mut c_void,
    pub wake: Option<extern "C" fn(*mut c_void)>,
    pub text_input: Option<extern "C" fn(*mut c_void, bool, NanaRect)>,
    pub pressed: Option<extern "C" fn(*mut c_void)>,
}

/// The callbacks with their context, callable from the task waker's thread.
#[derive(Clone, Copy)]
struct Callbacks(NanaIosCallbacks);

// SAFETY: the Swift shell's callbacks only dispatch onto the main queue, and
// its context object outlives the host.
unsafe impl Send for Callbacks {}
unsafe impl Sync for Callbacks {}

impl Callbacks {
    fn wake(self) {
        if let Some(wake) = self.0.wake {
            wake(self.0.context);
        }
    }

    fn pressed(self) {
        if let Some(pressed) = self.0.pressed {
            pressed(self.0.context);
        }
    }
}

struct Platform {
    callbacks: Callbacks,
    /// UIPasteboard stays with the shell's edit menu (UITextInteraction);
    /// in-document copy and paste go through this.
    clipboard: SharedClipboardHost,
}

impl MobilePlatform for Platform {
    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        let Some(callback) = self.callbacks.0.text_input else {
            return;
        };
        let caret = state
            .and_then(|state| state.cursor_area)
            .map(|area| NanaRect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height,
            })
            .unwrap_or_default();
        callback(self.callbacks.0.context, state.is_some(), caret);
    }

    fn clipboard(&self) -> SharedClipboardHost {
        self.clipboard.clone()
    }
}

pub struct NanaIosHost {
    runtime: MobileRuntime,
    renderer: MobileRenderer,
    probe: Probe,
    preview: PreviewUploader,
}

impl NanaIosHost {
    fn tick(&mut self, now: Instant) -> Result<FrameDemand, String> {
        self.probe
            .sync_preview(&mut self.preview, self.renderer.host_textures());
        let changed = self
            .runtime
            .prepare_frame(now)
            .map_err(|error| error.to_string())?;
        if changed || self.renderer.host_textures_changed() {
            self.renderer.render(&self.runtime)?;
        }
        Ok(self.demand())
    }

    fn demand(&self) -> FrameDemand {
        let demand = self.runtime.frame_demand();
        if self.renderer.host_textures_changed() {
            demand.merge(FrameDemand::Now)
        } else {
            demand
        }
    }
}

fn layer_target(layer: *mut c_void) -> wgpu::SurfaceTargetUnsafe {
    wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer)
}

/// # Safety
///
/// `layer` is a live `CAMetalLayer` the caller retains until
/// [`nana_ios_host_detach`] or [`nana_ios_host_free`].
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn nana_ios_host_new(
    layer: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    safe_area: NanaSafeArea,
    dark: bool,
    text_scale: f32,
    transparent: bool,
    callbacks: NanaIosCallbacks,
) -> *mut NanaIosHost {
    let callbacks = Callbacks(callbacks);
    nana_mobile_host::set_task_wake(move || callbacks.wake());
    let viewport = MobileViewport::new((width, height), scale, safe_area.into());
    let appearance = MobileAppearance { dark, text_scale };
    let platform = Box::new(Platform {
        callbacks,
        clipboard: shared_clipboard(MemoryClipboard::default()),
    });
    let Ok(mut runtime) = MobileRuntime::new(viewport, appearance, platform) else {
        return std::ptr::null_mut();
    };
    runtime.set_scroll_physics(ScrollPhysics::Ios);
    if transparent && runtime.set_background_passthrough(true).is_err() {
        return std::ptr::null_mut();
    }
    let state = ProbeState::with_press_feedback(move || callbacks.pressed());
    let Ok(probe) = Probe::mount(&mut runtime, state) else {
        return std::ptr::null_mut();
    };
    // SAFETY: forwarded from the caller.
    let renderer = unsafe {
        MobileRenderer::new(
            wgpu::Backends::METAL,
            layer_target(layer),
            (width, height),
            SurfaceOptions { transparent },
        )
    };
    let Ok(renderer) = renderer else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(NanaIosHost {
        runtime,
        renderer,
        probe,
        preview: PreviewUploader::default(),
    }))
}

/// # Safety
///
/// `host` came from [`nana_ios_host_new`] and is not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_free(host: *mut NanaIosHost) {
    if !host.is_null() {
        // SAFETY: the caller hands back the box it was given.
        drop(unsafe { Box::from_raw(host) });
    }
}

/// Borrow the host for one call.
///
/// # Safety
///
/// `host` is null or a live host from [`nana_ios_host_new`], used on the
/// main thread only.
unsafe fn host<'a>(host: *mut NanaIosHost) -> Option<&'a mut NanaIosHost> {
    // SAFETY: forwarded from the caller.
    unsafe { host.as_mut() }
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_resize(
    host: *mut NanaIosHost,
    width: u32,
    height: u32,
    scale: f32,
    safe_area: NanaSafeArea,
) {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return;
    };
    let viewport = MobileViewport::new((width, height), scale, safe_area.into());
    let _ = host.runtime.set_viewport(viewport);
    host.renderer.resize(viewport.physical_size);
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_set_appearance(
    host: *mut NanaIosHost,
    dark: bool,
    text_scale: f32,
) {
    // SAFETY: forwarded from the caller.
    if let Some(host) = unsafe { self::host(host) } {
        host.runtime
            .set_appearance(MobileAppearance { dark, text_scale });
    }
}

/// The page background for the current appearance, sRGB RGBA in `0..=1`:
/// a transparent host paints none, and the shell paints this under its
/// native layers instead.
///
/// # Safety
///
/// See [`host`]; `rgba` points at four writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_background(host: *mut NanaIosHost, rgba: *mut f32) -> bool {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return false;
    };
    let Ok(color) = host.runtime.background_rgba() else {
        return false;
    };
    if rgba.is_null() {
        return false;
    }
    // SAFETY: the caller passes four writable floats.
    unsafe { std::ptr::copy_nonoverlapping(color.as_ptr(), rgba, 4) };
    true
}

/// # Safety
///
/// See [`host`]; `layer` as in [`nana_ios_host_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_attach(
    host: *mut NanaIosHost,
    layer: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return false;
    };
    // SAFETY: forwarded from the caller.
    let attached = unsafe { host.renderer.attach(layer_target(layer), (width, height)) };
    host.runtime.request_present();
    attached.is_ok()
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_detach(host: *mut NanaIosHost) {
    // SAFETY: forwarded from the caller.
    if let Some(host) = unsafe { self::host(host) } {
        host.renderer.detach();
    }
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_touch(
    host: *mut NanaIosHost,
    phase: i32,
    x: f32,
    y: f32,
    id: u64,
) -> bool {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return false;
    };
    let phase = match phase {
        0 => TouchPhase::Down,
        1 => TouchPhase::Move,
        2 => TouchPhase::Up,
        _ => TouchPhase::Cancel,
    };
    host.runtime.push_touch(phase, x, y, id).unwrap_or(false)
}

fn write_demand(demand: FrameDemand, now: Instant, seconds: *mut f64) -> i32 {
    match demand {
        FrameDemand::Idle => 0,
        FrameDemand::Now => 1,
        FrameDemand::At(deadline) => {
            if !seconds.is_null() {
                let delay = deadline.saturating_duration_since(now).as_secs_f64();
                // SAFETY: the caller passes a writable double or null.
                unsafe { *seconds = delay };
            }
            2
        }
    }
}

/// # Safety
///
/// See [`host`]; `seconds` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_tick(host: *mut NanaIosHost, seconds: *mut f64) -> i32 {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return -1;
    };
    let now = Instant::now();
    match host.tick(now) {
        Ok(demand) => write_demand(demand.resolve(now), now, seconds),
        Err(error) => {
            // Stderr reaches the Xcode console and `simctl launch --console`.
            eprintln!("nana-ios-host: tick failed: {error}");
            -1
        }
    }
}

/// # Safety
///
/// See [`host`]; `seconds` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_frame_demand(
    host: *mut NanaIosHost,
    seconds: *mut f64,
) -> i32 {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return -1;
    };
    let now = Instant::now();
    write_demand(host.demand().resolve(now), now, seconds)
}

/// # Safety
///
/// `text` points at `len` bytes of UTF-8 (or is null with `len` 0).
unsafe fn utf8(text: *const u8, len: usize) -> Option<String> {
    if text.is_null() {
        return (len == 0).then(String::new);
    }
    // SAFETY: forwarded from the caller.
    let bytes = unsafe { std::slice::from_raw_parts(text, len) };
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// # Safety
///
/// See [`host`] and [`utf8`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_set_marked_text(
    host: *mut NanaIosHost,
    text: *const u8,
    len: usize,
    selection_start: i64,
    selection_end: i64,
) {
    // SAFETY: forwarded from the caller.
    let (Some(host), Some(text)) = (unsafe { self::host(host) }, unsafe { utf8(text, len) }) else {
        return;
    };
    let selection = (selection_start >= 0 && selection_end >= selection_start)
        .then_some((selection_start as usize, selection_end as usize));
    let event = if text.is_empty() {
        CompositionInput::End
    } else {
        CompositionInput::Update { text, selection }
    };
    let _ = host.runtime.push_composition(event);
}

/// # Safety
///
/// See [`host`] and [`utf8`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_commit_text(
    host: *mut NanaIosHost,
    text: *const u8,
    len: usize,
) {
    // SAFETY: forwarded from the caller.
    let (Some(host), Some(text)) = (unsafe { self::host(host) }, unsafe { utf8(text, len) }) else {
        return;
    };
    let _ = host
        .runtime
        .push_composition(CompositionInput::Commit(text));
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_delete_surrounding(
    host: *mut NanaIosHost,
    before: usize,
    after: usize,
) {
    // SAFETY: forwarded from the caller.
    if let Some(host) = unsafe { self::host(host) } {
        let _ = host
            .runtime
            .push_composition(CompositionInput::DeleteSurrounding {
                before_bytes: before,
                after_bytes: after,
            });
    }
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_set_selection(
    host: *mut NanaIosHost,
    anchor: usize,
    focus: usize,
) {
    // SAFETY: forwarded from the caller.
    if let Some(host) = unsafe { self::host(host) } {
        let _ = host.runtime.set_text_selection(anchor, focus);
    }
}

fn write_offset(out: *mut i64, value: Option<usize>) {
    if !out.is_null() {
        // SAFETY: the caller passes writable integers or null.
        unsafe { *out = value.map_or(-1, |value| value as i64) };
    }
}

/// # Safety
///
/// See [`host`]; `buffer` holds `capacity` bytes, the offsets are writable
/// or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_text_snapshot(
    host: *mut NanaIosHost,
    buffer: *mut u8,
    capacity: usize,
    selection_start: *mut i64,
    selection_end: *mut i64,
    marked_start: *mut i64,
    marked_end: *mut i64,
) -> i64 {
    // SAFETY: forwarded from the caller.
    let snapshot = unsafe { self::host(host) }.and_then(|host| host.runtime.text_input());
    let Some(snapshot) = snapshot else {
        for out in [selection_start, selection_end, marked_start, marked_end] {
            write_offset(out, None);
        }
        return -1;
    };
    let bytes = snapshot.text.as_bytes();
    if !buffer.is_null() && bytes.len() <= capacity {
        // SAFETY: `buffer` holds `capacity >= bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len()) };
    }
    write_offset(selection_start, Some(snapshot.selection.0));
    write_offset(selection_end, Some(snapshot.selection.1));
    write_offset(marked_start, snapshot.marked.map(|marked| marked.0));
    write_offset(marked_end, snapshot.marked.map(|marked| marked.1));
    bytes.len() as i64
}

fn write_rect(out: *mut NanaRect, rect: LogicalRect) {
    if !out.is_null() {
        // SAFETY: the caller passes a writable rect or null.
        unsafe {
            *out = NanaRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            };
        }
    }
}

/// # Safety
///
/// See [`host`]; `rect` is writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_text_bounds(
    host: *mut NanaIosHost,
    rect: *mut NanaRect,
) -> bool {
    // SAFETY: forwarded from the caller.
    let bounds = unsafe { self::host(host) }.and_then(|host| host.runtime.text_input_bounds());
    bounds.map(|bounds| write_rect(rect, bounds)).is_some()
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_text_offset_at(
    host: *mut NanaIosHost,
    x: f32,
    y: f32,
) -> i64 {
    // SAFETY: forwarded from the caller.
    unsafe { self::host(host) }
        .and_then(|host| host.runtime.text_offset_at(x, y))
        .map_or(-1, |offset| offset as i64)
}

/// # Safety
///
/// See [`host`]; `rect` is writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_text_caret_rect(
    host: *mut NanaIosHost,
    offset: usize,
    rect: *mut NanaRect,
) -> bool {
    // SAFETY: forwarded from the caller.
    let caret = unsafe { self::host(host) }.and_then(|host| host.runtime.text_caret_rect(offset));
    caret.map(|caret| write_rect(rect, caret)).is_some()
}

/// # Safety
///
/// See [`host`]; `rect` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_preview_rect(
    host: *mut NanaIosHost,
    rect: *mut NanaRect,
) -> bool {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return false;
    };
    let Some(area) = host.probe.preview_rect(&host.runtime) else {
        return false;
    };
    write_rect(rect, area);
    true
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_cpu_preview(host: *mut NanaIosHost) -> bool {
    // SAFETY: forwarded from the caller.
    unsafe { self::host(host) }.is_some_and(|host| host.probe.state.cpu_preview())
}

/// # Safety
///
/// See [`host`]; `bytes` holds `bytes_per_row * height` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_push_preview_bgra(
    host: *mut NanaIosHost,
    bytes: *const u8,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return;
    };
    if !host.probe.state.cpu_preview() || bytes.is_null() {
        return;
    }
    // SAFETY: forwarded from the caller.
    let bytes =
        unsafe { std::slice::from_raw_parts(bytes, (bytes_per_row as usize) * height as usize) };
    let gpu = host.renderer.gpu();
    let textures = host.renderer.host_textures();
    if host
        .preview
        .push_bgra(gpu, textures, bytes, width, height, bytes_per_row)
        .is_ok()
    {
        host.runtime.request_present();
    }
}

/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_presses(host: *mut NanaIosHost) -> u32 {
    // SAFETY: forwarded from the caller.
    unsafe { self::host(host) }.map_or(0, |host| host.probe.state.presses())
}
