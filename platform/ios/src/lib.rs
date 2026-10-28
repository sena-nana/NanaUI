//! Experimental iOS host (`include/nana_ios_host.h`).
//!
//! The Swift shell (`platform/ios/Shell`) owns the UIKit view, its
//! `CAMetalLayer`, the `CADisplayLink`, the `UITextInput` proxy and any
//! native layers around the surface; it lowers UIKit events into these
//! calls. The document, input routing, frame demand and the device live in
//! [`nana_mobile_host`], shared with the Android host. Every call is made on
//! the main thread; the callbacks never call back in synchronously.
//!
//! The host is an rlib: an app crate builds the one staticlib the Xcode
//! target links, puts its view in through [`IosApp`], exports the shared
//! `nana_ios_host_new` with [`export_ios_app!`], and adds its own C calls
//! through [`with_app`].

use std::any::Any;
use std::ffi::c_void;
use std::time::Instant;

use nana_mobile_host::{
    FrameDemand, MobileAppearance, MobilePlatform, MobileRenderer, MobileRuntime, MobileViewport,
    SafeArea, ScrollPhysics, SurfaceOptions, TouchPhase,
};
pub use nana_ui::runtime::FrameworkError;
use nana_ui::runtime::{AccessibilityAction, StableNodeId};
use nana_ui::{AccessibilityActionRequest, AccessibilityNode, AccessibilityRole};
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
    /// One `NANA_HAPTIC_*` style.
    pub haptic: Option<extern "C" fn(*mut c_void, i32)>,
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

    fn haptic(self, style: Haptic) {
        if let Some(haptic) = self.0.haptic {
            haptic(self.0.context, style as i32);
        }
    }
}

/// Feedback the shell plays through its `UIFeedbackGenerator`s.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    /// A light impact: a button press.
    Light = 0,
    /// A medium impact.
    Medium = 1,
    /// A selection tick: a value stepping.
    Selection = 2,
    /// A success notification.
    Success = 3,
    /// A warning notification.
    Warning = 4,
}

/// What an app reaches the shell through, from any thread.
#[derive(Clone, Copy)]
pub struct Shell(Callbacks);

impl Shell {
    pub fn haptic(&self, style: Haptic) {
        self.0.haptic(style);
    }

    /// Ask the shell for a frame: state the document reads changed off the
    /// main thread.
    pub fn wake(&self) {
        self.0.wake();
    }
}

/// The application a host shows: it mounts its view once, and may prepare
/// host textures or other per-frame state before each frame.
pub trait IosApp: Any {
    fn mount(&mut self, runtime: &mut MobileRuntime, shell: Shell) -> Result<(), FrameworkError>;

    /// Runs on the main thread before each frame is prepared.
    fn before_frame(&mut self, _runtime: &mut MobileRuntime, _renderer: &mut MobileRenderer) {}
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
    app: Box<dyn IosApp>,
    /// The accessibility nodes the shell last asked for, read by index.
    accessibility: Vec<AccessibilityNode>,
}

/// The parts of a live host an app's own C calls work with.
pub struct HostParts<'a, A> {
    pub app: &'a mut A,
    pub runtime: &'a mut MobileRuntime,
    pub renderer: &'a mut MobileRenderer,
}

/// Borrow the host's app as `A` for one of the app's own C calls; `None` for
/// a null host or another app type.
///
/// # Safety
///
/// `host` is null or a live host from `nana_ios_host_new`, used on the main
/// thread only.
pub unsafe fn with_app<A: IosApp, R>(
    host: *mut NanaIosHost,
    f: impl FnOnce(HostParts<'_, A>) -> R,
) -> Option<R> {
    // SAFETY: forwarded from the caller.
    let host = unsafe { self::host(host) }?;
    let app = (host.app.as_mut() as &mut dyn Any).downcast_mut::<A>()?;
    Some(f(HostParts {
        app,
        runtime: &mut host.runtime,
        renderer: &mut host.renderer,
    }))
}

impl NanaIosHost {
    fn tick(&mut self, now: Instant) -> Result<FrameDemand, String> {
        self.app.before_frame(&mut self.runtime, &mut self.renderer);
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

/// Export `nana_ios_host_new`, building the host around `$app` (an
/// expression of a type implementing [`IosApp`]). Use it once, in the crate
/// that builds the staticlib.
#[macro_export]
macro_rules! export_ios_app {
    ($app:expr) => {
        /// # Safety
        ///
        /// `layer` is a live `CAMetalLayer` the caller retains until
        /// `nana_ios_host_detach` or `nana_ios_host_free`.
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub unsafe extern "C" fn nana_ios_host_new(
            layer: *mut ::std::ffi::c_void,
            width: u32,
            height: u32,
            scale: f32,
            safe_area: $crate::NanaSafeArea,
            dark: bool,
            text_scale: f32,
            transparent: bool,
            callbacks: $crate::NanaIosCallbacks,
        ) -> *mut $crate::NanaIosHost {
            // SAFETY: forwarded from the caller.
            unsafe {
                $crate::create(
                    layer,
                    width,
                    height,
                    scale,
                    safe_area,
                    dark,
                    text_scale,
                    transparent,
                    callbacks,
                    ::std::boxed::Box::new($app),
                )
            }
        }
    };
}

/// Build a host around `app`; the body of [`export_ios_app!`].
///
/// # Safety
///
/// `layer` is a live `CAMetalLayer` the caller retains until
/// [`nana_ios_host_detach`] or [`nana_ios_host_free`].
#[allow(clippy::too_many_arguments)]
pub unsafe fn create(
    layer: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    safe_area: NanaSafeArea,
    dark: bool,
    text_scale: f32,
    transparent: bool,
    callbacks: NanaIosCallbacks,
    mut app: Box<dyn IosApp>,
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
    if app.mount(&mut runtime, Shell(callbacks)).is_err() {
        return std::ptr::null_mut();
    }
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
        app,
        accessibility: Vec::new(),
    }))
}

/// # Safety
///
/// `host` came from `nana_ios_host_new` and is not used again.
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
/// `host` is null or a live host from `nana_ios_host_new`, used on the
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
/// See [`host`]; `layer` as in `nana_ios_host_new`.
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

/// One node of the flat accessibility snapshot, logical px of the surface.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NanaAccessibilityNode {
    pub id: u64,
    /// One `NANA_AX_ROLE_*`.
    pub role: i32,
    /// `NANA_AX_*` state bits.
    pub flags: u32,
    pub frame: NanaRect,
    /// The numeric value of a slider; NaN for other nodes.
    pub numeric_value: f64,
}

const AX_DISABLED: u32 = 1;
const AX_CHECKED: u32 = 1 << 1;
const AX_SELECTED: u32 = 1 << 2;
const AX_FOCUSED: u32 = 1 << 3;
const AX_EDITABLE: u32 = 1 << 4;
const AX_BUSY: u32 = 1 << 5;

/// The roles the shell gives traits to; the rest read as plain elements.
fn ax_role(role: AccessibilityRole) -> Option<i32> {
    Some(match role {
        AccessibilityRole::Text | AccessibilityRole::Status => 1,
        AccessibilityRole::Button
        | AccessibilityRole::MenuItem
        | AccessibilityRole::ListBoxOption
        | AccessibilityRole::ComboBox => 2,
        AccessibilityRole::TextInput | AccessibilityRole::Search => 3,
        AccessibilityRole::Switch | AccessibilityRole::Checkbox | AccessibilityRole::Radio => 4,
        AccessibilityRole::Slider => 5,
        AccessibilityRole::Image => 6,
        AccessibilityRole::Tab => 7,
        AccessibilityRole::ListItem | AccessibilityRole::TreeItem | AccessibilityRole::Row => 8,
        AccessibilityRole::ProgressIndicator => 9,
        _ => return None,
    })
}

/// Whether VoiceOver should stop on the node: a role it announces, or any
/// labelled node.
fn ax_exposed(node: &AccessibilityNode) -> bool {
    ax_role(node.role).is_some() || node.label.as_deref().is_some_and(|label| !label.is_empty())
}

/// Take a fresh flat snapshot of the nodes VoiceOver stops on, in document
/// order, and return how many there are; read them with
/// [`nana_ios_host_accessibility_node`] and
/// [`nana_ios_host_accessibility_text`].
///
/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_accessibility_count(host: *mut NanaIosHost) -> usize {
    // SAFETY: forwarded from the caller.
    let Some(host) = (unsafe { self::host(host) }) else {
        return 0;
    };
    host.accessibility = host
        .runtime
        .accessibility_nodes()
        .into_iter()
        .filter(ax_exposed)
        .collect();
    host.accessibility.len()
}

/// # Safety
///
/// See [`host`]; `node` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_accessibility_node(
    host: *mut NanaIosHost,
    index: usize,
    node: *mut NanaAccessibilityNode,
) -> bool {
    // SAFETY: forwarded from the caller.
    let Some(source) = (unsafe { self::host(host) }).and_then(|host| host.accessibility.get(index))
    else {
        return false;
    };
    if node.is_null() {
        return false;
    }
    let mut flags = 0;
    for (on, bit) in [
        (source.disabled, AX_DISABLED),
        (source.checked == Some(true), AX_CHECKED),
        (source.selected == Some(true), AX_SELECTED),
        (source.focused, AX_FOCUSED),
        (source.editable, AX_EDITABLE),
        (source.busy, AX_BUSY),
    ] {
        if on {
            flags |= bit;
        }
    }
    let bounds = source.bounds;
    // SAFETY: the caller passes a writable node.
    unsafe {
        *node = NanaAccessibilityNode {
            id: source.id.get(),
            role: ax_role(source.role).unwrap_or(0),
            flags,
            frame: NanaRect {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            numeric_value: source.numeric_value.unwrap_or(f64::NAN),
        };
    }
    true
}

/// Copy one text of a snapshot node (`which`: 0 label, 1 value, 2 hint) into
/// `buffer` when it fits; its UTF-8 length, or -1 when the node has none.
///
/// # Safety
///
/// See [`host`]; `buffer` holds `capacity` bytes or is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_accessibility_text(
    host: *mut NanaIosHost,
    index: usize,
    which: i32,
    buffer: *mut u8,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded from the caller.
    let Some(node) = (unsafe { self::host(host) }).and_then(|host| host.accessibility.get(index))
    else {
        return -1;
    };
    let text: Option<&str> = match which {
        0 => node.label.as_deref(),
        1 => node.value.as_deref(),
        _ => node.description.as_deref(),
    };
    let Some(text) = text else {
        return -1;
    };
    let bytes = text.as_bytes();
    if !buffer.is_null() && bytes.len() <= capacity {
        // SAFETY: `buffer` holds `capacity >= bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len()) };
    }
    bytes.len() as i64
}

/// Perform a VoiceOver action (`NANA_AX_ACTION_*`) on node `id`.
///
/// # Safety
///
/// See [`host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_ios_host_accessibility_action(
    host: *mut NanaIosHost,
    id: u64,
    action: i32,
) -> bool {
    // SAFETY: forwarded from the caller.
    let (Some(host), Some(target)) = (unsafe { self::host(host) }, StableNodeId::new(id)) else {
        return false;
    };
    let action = match action {
        0 => AccessibilityAction::Click,
        1 => AccessibilityAction::Increment,
        2 => AccessibilityAction::Decrement,
        _ => AccessibilityAction::Focus,
    };
    host.runtime
        .apply_accessibility_action(AccessibilityActionRequest { target, action })
        .unwrap_or(false)
}
