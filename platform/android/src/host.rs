//! Android activity loop over [`nana_mobile_host`]: window lifecycle → the
//! shared renderer, GameActivity input → the shared runtime, and the loop
//! sleeping whenever the document asks for no frame.
//!
//! Frames: while the document demands one, the loop presents each pass and
//! the Fifo swapchain paces it to vsync; a deadline sleeps in
//! `poll_events` until it is due; an idle document blocks in `poll_events`
//! until input, a lifecycle event or a view task wakes it. Nothing presents
//! on a timer.
//!
//! `TerminateWindow` (background) drops only the surface: the document, its
//! state and the device survive until the next `InitWindow`.

use std::time::{Duration, Instant};

use android_activity::input::{
    ImeOptions, InputEvent, InputType, KeyAction, Keycode, MetaState, MotionAction,
    TextInputAction, TextInputState, TextSpan,
};
use android_activity::ndk::configuration::UiModeNight;
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use nana_mobile_host::{
    FrameDemand, MobileAppearance, MobilePlatform, MobileRenderer, MobileRuntime, MobileViewport,
    SafeArea, ScrollPhysics, SurfaceOptions, TouchPhase,
};
use nana_mobile_probe::{PreviewUploader, Probe, ProbeState};
use nana_ui_platform::{SharedClipboardHost, TextInputContext, default_shared_clipboard};
use raw_window_handle::{AndroidNdkWindowHandle, DisplayHandle, RawWindowHandle, WindowHandle};

use crate::shell::{SystemInsets, primary_rect, scale_factor_from_density_dpi};
use crate::slot_ax::SlotAccessibility;
use crate::slot_ime::{SlotEditorInfo, SlotImeBuffer, ime_events_from_buffer_delta};
use crate::slot_input::{
    SlotInputGate, SlotKeyDispatch, SlotKeyMods, SlotLogicalKey, SlotTouchKind,
    android_keycode_is_modifier, host_swallows_for_input_connection,
    logical_key_from_android_keycode, slot_key_to_dispatch,
};

/// The CPU preview stand-in: Android has no camera in the prototype, so a
/// generated 640×480 frame is uploaded at the camera's 30 fps to measure the
/// upload path the iOS host compares against its native layer.
const PREVIEW_SIZE: (u32, u32) = (640, 480);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(33);

struct AndroidPlatform;

impl MobilePlatform for AndroidPlatform {
    // The loop mirrors the focused editor into GameTextInput itself each
    // pass ([`Host::sync_soft_input`]).
    fn set_text_input(&mut self, _state: Option<&TextInputContext>) {}

    fn clipboard(&self) -> SharedClipboardHost {
        default_shared_clipboard()
    }
}

/// What lives from the first window to `Destroy`.
struct Document {
    runtime: MobileRuntime,
    renderer: MobileRenderer,
    probe: Probe,
    preview: PreviewUploader,
    ax: Option<SlotAccessibility>,
}

struct Host {
    document: Option<Document>,
    gate: SlotInputGate,
    insets: SystemInsets,
    ime_shown: bool,
    ime_buffer: SlotImeBuffer,
    editor_info: Option<SlotEditorInfo>,
    next_preview: Instant,
    preview_phase: u8,
}

impl Host {
    fn new() -> Self {
        Self {
            document: None,
            gate: SlotInputGate::default(),
            insets: SystemInsets::default(),
            ime_shown: false,
            ime_buffer: SlotImeBuffer::default(),
            editor_info: None,
            next_preview: Instant::now(),
            preview_phase: 0,
        }
    }

    fn attached(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.renderer.is_attached())
    }

    fn viewport(&mut self, app: &AndroidApp) -> MobileViewport {
        let physical = window_size(app);
        let scale = scale_factor_from_density_dpi(app.config().density());
        let safe = safe_area(app);
        self.insets = SystemInsets {
            top: safe.top as u32,
            bottom: safe.bottom as u32,
        };
        MobileViewport::new(physical, scale, safe)
    }

    fn appearance(app: &AndroidApp) -> MobileAppearance {
        MobileAppearance {
            dark: app.config().ui_mode_night() == UiModeNight::Yes,
            text_scale: font_scale(app),
        }
    }

    fn on_window_ready(&mut self, app: &AndroidApp) -> Result<(), String> {
        let Some(window) = app.native_window() else {
            return Err("ANativeWindow missing".into());
        };
        let viewport = self.viewport(app);
        let display = DisplayHandle::android();
        let handle = AndroidNdkWindowHandle::new(window.ptr().cast());
        // SAFETY: the ANativeWindow lives until TerminateWindow, where the
        // surface is dropped first.
        let target = unsafe {
            let window = WindowHandle::borrow_raw(RawWindowHandle::AndroidNdk(handle));
            wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display.as_raw()),
                raw_window_handle: window.as_raw(),
            }
        };
        if let Some(document) = self.document.as_mut() {
            // SAFETY: as above.
            unsafe { document.renderer.attach(target, viewport.physical_size) }?;
            document
                .runtime
                .set_viewport(viewport)
                .map_err(|error| error.to_string())?;
            document.runtime.request_present();
            log::info!("nana-android-host: surface reattached, document kept");
            return Ok(());
        }
        let mut runtime =
            MobileRuntime::new(viewport, Self::appearance(app), Box::new(AndroidPlatform))
                .map_err(|error| format!("runtime: {error}"))?;
        runtime.set_scroll_physics(ScrollPhysics::Android);
        let probe = Probe::mount(&mut runtime, ProbeState::default())
            .map_err(|error| format!("probe: {error}"))?;
        // SAFETY: as above.
        let renderer = unsafe {
            MobileRenderer::new(
                wgpu::Backends::VULKAN,
                target,
                viewport.physical_size,
                SurfaceOptions::default(),
            )
        }?;
        let ax = SlotAccessibility::new(app, &runtime)
            .inspect_err(|error| {
                log::warn!("nana-android-host: accessibility attach failed: {error}")
            })
            .ok();
        self.document = Some(Document {
            runtime,
            renderer,
            probe,
            preview: PreviewUploader::default(),
            ax,
        });
        log::info!("nana-android-host: document ready scale={}", viewport.scale);
        Ok(())
    }

    fn on_geometry_changed(&mut self, app: &AndroidApp) {
        let viewport = self.viewport(app);
        let appearance = Self::appearance(app);
        let Some(document) = self.document.as_mut() else {
            return;
        };
        document.renderer.resize(viewport.physical_size);
        if let Err(error) = document.runtime.set_viewport(viewport) {
            log::warn!("nana-android-host: viewport: {error}");
        }
        document.runtime.set_appearance(appearance);
    }

    fn on_window_destroyed(&mut self) {
        if let Some(document) = self.document.as_mut() {
            document.renderer.detach();
        }
        self.ime_shown = false;
        self.ime_buffer = SlotImeBuffer::default();
        self.editor_info = None;
        log::info!("nana-android-host: surface destroyed, document kept");
    }

    /// When the loop should come back without input.
    fn demand(&self) -> FrameDemand {
        let Some(document) = self.document.as_ref().filter(|_| self.attached()) else {
            return FrameDemand::Idle;
        };
        let mut demand = document.runtime.frame_demand();
        if document.renderer.host_textures_changed() {
            demand = demand.merge(FrameDemand::Now);
        }
        if document.probe.state.cpu_preview() {
            demand = demand.merge(FrameDemand::At(self.next_preview));
        }
        demand
    }

    /// One pass's frame, when it is due.
    fn frame(&mut self, now: Instant) {
        if !self.attached() {
            return;
        }
        self.feed_preview(now);
        if let Some(document) = self.document.as_mut() {
            document
                .probe
                .sync_preview(&mut document.preview, document.renderer.host_textures());
        }
        if self.demand().resolve(now) != FrameDemand::Now {
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        match document.runtime.prepare_frame(now) {
            Ok(changed) if changed || document.renderer.host_textures_changed() => {
                if let Err(error) = document.renderer.render(&document.runtime) {
                    log::warn!("nana-android-host: render: {error}");
                }
                if let Some(ax) = document.ax.as_mut() {
                    ax.push(&document.runtime);
                }
            }
            Ok(_) => {}
            Err(error) => log::warn!("nana-android-host: frame: {error}"),
        }
    }

    fn feed_preview(&mut self, now: Instant) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if !document.probe.state.cpu_preview() || now < self.next_preview {
            return;
        }
        self.next_preview = now + PREVIEW_INTERVAL;
        self.preview_phase = self.preview_phase.wrapping_add(3);
        let (width, height) = PREVIEW_SIZE;
        let shade = self.preview_phase;
        let frame: Vec<u8> = (0..width * height)
            .flat_map(|index| {
                let x = (index % width) as u8;
                [x.wrapping_add(shade), shade, 160, 255]
            })
            .collect();
        let uploaded = document.preview.push_bgra(
            document.renderer.gpu(),
            document.renderer.host_textures(),
            &frame,
            width,
            height,
            width * 4,
        );
        if uploaded.is_ok() {
            document.runtime.request_present();
        }
    }

    fn drain_accessibility_actions(&mut self) {
        if let Some(document) = self.document.as_mut()
            && let Some(ax) = document.ax.as_mut()
        {
            ax.drain_actions(&mut document.runtime);
        }
    }

    fn text_input_focused(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.runtime.text_input().is_some())
    }

    fn ime_buffer(&self) -> Option<SlotImeBuffer> {
        let snapshot = self.document.as_ref()?.runtime.text_input()?;
        Some(SlotImeBuffer {
            text: snapshot.text,
            selection_start: snapshot.selection.0,
            selection_end: snapshot.selection.1,
            compose: snapshot.marked,
        })
    }

    /// Mirror the focused editor onto GameTextInput and the soft keyboard.
    fn sync_soft_input(&mut self, app: &AndroidApp) {
        let focused = self.text_input_focused();
        if focused {
            let info = crate::slot_ime::editor_info_from_request(false, false);
            if self.editor_info != Some(info) {
                apply_editor_info(app, info);
                self.editor_info = Some(info);
            }
            if let Some(buffer) = self.ime_buffer()
                && buffer != self.ime_buffer
            {
                set_text_input_state(app, &buffer);
                self.ime_buffer = buffer;
            }
        } else {
            self.editor_info = None;
        }
        match (focused, self.ime_shown) {
            (true, false) => {
                app.show_soft_input(true);
                self.ime_shown = true;
            }
            (false, true) => {
                app.hide_soft_input(false);
                self.ime_shown = false;
                self.ime_buffer = SlotImeBuffer::default();
            }
            _ => {}
        }
    }

    fn handle_motion(&mut self, action: MotionAction, x: f32, y: f32, pointer_id: i32) -> bool {
        let (kind, phase) = match action {
            MotionAction::Down | MotionAction::PointerDown => {
                (SlotTouchKind::Down, TouchPhase::Down)
            }
            MotionAction::Move => (SlotTouchKind::Move, TouchPhase::Move),
            MotionAction::Up | MotionAction::PointerUp => (SlotTouchKind::Up, TouchPhase::Up),
            MotionAction::Cancel => (SlotTouchKind::Cancel, TouchPhase::Cancel),
            _ => return false,
        };
        let Some(document) = self.document.as_mut() else {
            return false;
        };
        let primary = primary_rect(document.runtime.viewport().physical_size, self.insets);
        // Only samples in Primary (or a captured drag) are Handled; the
        // status and navigation bars stay with the system.
        if !self
            .gate
            .accept_pointer(Some(primary), kind, x, y, pointer_id)
        {
            return false;
        }
        document
            .runtime
            .push_touch(phase, x, y, pointer_id.max(0) as u64)
            .unwrap_or(false)
    }

    /// GameActivity KeyEvent → Runtime keyboard. System keys (Back, …) stay
    /// `Unhandled`; printable commits while editing arrive as TextEvents.
    fn handle_key(
        &mut self,
        action: KeyAction,
        keycode: Keycode,
        meta: MetaState,
        repeat: i32,
    ) -> bool {
        let down = match action {
            KeyAction::Down | KeyAction::Multiple => true,
            KeyAction::Up => false,
            _ => return false,
        };
        let keycode: u32 = keycode.into();
        let mods = SlotKeyMods {
            shift: meta.shift_on(),
            ctrl: meta.ctrl_on(),
            alt: meta.alt_on(),
            logo: meta.meta_on(),
        };
        let logical = logical_key_from_android_keycode(keycode, mods.shift, meta.caps_lock_on());
        if logical.is_none() && !android_keycode_is_modifier(keycode) {
            return false;
        }
        let focused = self.text_input_focused();
        if host_swallows_for_input_connection(down, self.gate.accept_key(), focused, logical, mods)
        {
            return true;
        }
        if !self.gate.accept_key() {
            return false;
        }
        let (Some(key), Some(document)) = (logical, self.document.as_mut()) else {
            return true;
        };
        let routed = match slot_key_to_dispatch(down, key, mods, down && repeat > 0, focused) {
            SlotKeyDispatch::Key { key, text } => document.runtime.push_key(key, text),
            SlotKeyDispatch::Ime(event) => document.runtime.push_composition(event),
        };
        routed.is_ok()
    }

    /// GameTextInput state change → composition events.
    fn handle_text_event(&mut self, state: &TextInputState) -> bool {
        if !self.text_input_focused() {
            return false;
        }
        let next = SlotImeBuffer::from_utf16(
            state.text.clone(),
            (state.selection.start, state.selection.end),
            state.compose_region.map(|span| (span.start, span.end)),
        );
        let events = ime_events_from_buffer_delta(&self.ime_buffer, &next);
        let Some(document) = self.document.as_mut() else {
            return false;
        };
        for event in &events {
            let _ = document.runtime.push_composition(event.clone());
        }
        self.ime_buffer = self.ime_buffer().unwrap_or(next);
        !events.is_empty()
    }

    fn handle_text_action(&mut self, action: TextInputAction) -> bool {
        if !matches!(
            action,
            TextInputAction::Done
                | TextInputAction::Go
                | TextInputAction::Send
                | TextInputAction::Next
        ) || !self.text_input_focused()
        {
            return false;
        }
        let Some(document) = self.document.as_mut() else {
            return false;
        };
        let key = crate::slot_input::key_to_input_event(
            true,
            SlotLogicalKey::Enter,
            SlotKeyMods::default().to_input(),
            false,
        );
        document.runtime.push_key(key, None).is_ok()
    }
}

pub fn run(app: AndroidApp) -> Result<(), String> {
    let waker = app.create_waker();
    nana_mobile_host::set_task_wake(move || waker.wake());
    let mut host = Host::new();
    let mut running = true;

    while running {
        let now = Instant::now();
        let timeout = match host.demand().resolve(now) {
            FrameDemand::Now => Some(Duration::ZERO),
            FrameDemand::At(deadline) => Some(deadline.saturating_duration_since(now)),
            FrameDemand::Idle => None,
        };
        app.poll_events(timeout, |event| {
            let PollEvent::Main(main) = event else {
                return;
            };
            match main {
                MainEvent::InitWindow { .. } => {
                    if let Err(error) = host.on_window_ready(&app) {
                        log::error!("nana-android-host: window ready failed: {error}");
                    }
                }
                MainEvent::TerminateWindow { .. } => host.on_window_destroyed(),
                MainEvent::Destroy => {
                    host.document = None;
                    running = false;
                }
                MainEvent::WindowResized { .. }
                | MainEvent::RedrawNeeded { .. }
                | MainEvent::InsetsChanged { .. }
                | MainEvent::ConfigChanged { .. } => {
                    host.on_geometry_changed(&app);
                    if let Some(document) = host.document.as_mut() {
                        document.runtime.request_present();
                    }
                }
                MainEvent::Pause | MainEvent::Stop => {
                    // Android dismisses the IME itself; the next focus
                    // re-shows it.
                    host.ime_shown = false;
                }
                _ => {}
            }
        });

        if let Ok(mut iter) = app.input_events_iter() {
            while iter.next(|event| {
                let handled = match event {
                    InputEvent::MotionEvent(motion) => {
                        let action = motion.action();
                        let pointer = motion.pointer_at_index(motion.pointer_index());
                        let handled = host.handle_motion(
                            action,
                            pointer.x(),
                            pointer.y(),
                            pointer.pointer_id(),
                        );
                        // Re-ask on every tap on the focused field: Back may
                        // have dismissed the IME since focus last changed.
                        if handled
                            && matches!(action, MotionAction::Up | MotionAction::PointerUp)
                            && host.text_input_focused()
                        {
                            app.show_soft_input(true);
                            host.ime_shown = true;
                        }
                        handled
                    }
                    InputEvent::KeyEvent(key) => host.handle_key(
                        key.action(),
                        key.key_code(),
                        key.meta_state(),
                        key.repeat_count(),
                    ),
                    InputEvent::TextEvent(text) => {
                        let handled = host.handle_text_event(text);
                        if handled && let Some(buffer) = host.ime_buffer() {
                            set_text_input_state(&app, &buffer);
                        }
                        handled
                    }
                    InputEvent::TextAction(action) => host.handle_text_action(*action),
                    _ => false,
                };
                if handled {
                    InputStatus::Handled
                } else {
                    InputStatus::Unhandled
                }
            }) {}
        }
        host.drain_accessibility_actions();
        host.frame(Instant::now());
        host.sync_soft_input(&app);
    }
    Ok(())
}

fn window_size(app: &AndroidApp) -> (u32, u32) {
    app.native_window()
        .map(|window| (window.width().max(1) as u32, window.height().max(1) as u32))
        .unwrap_or((720, 1280))
}

fn set_text_input_state(app: &AndroidApp, buffer: &SlotImeBuffer) {
    let ((start, end), compose) = buffer.utf16_spans();
    app.set_text_input_state(TextInputState {
        text: buffer.text.clone(),
        selection: TextSpan { start, end },
        compose_region: compose.map(|(start, end)| TextSpan { start, end }),
    });
}

fn apply_editor_info(app: &AndroidApp, info: SlotEditorInfo) {
    let input_type = InputType::from_bits_truncate(info.input_type);
    let action = if info.action == crate::slot_ime::IME_ACTION_NONE {
        TextInputAction::None
    } else {
        TextInputAction::Done
    };
    let options = ImeOptions::from_bits_truncate(info.ime_options);
    app.set_ime_editor_info(input_type, action, options);
}

/// Run `read` against the Activity on this thread's JNI env; `None` (and a
/// warning) when any call fails.
fn with_activity<T>(
    app: &AndroidApp,
    what: &str,
    read: impl FnOnce(
        &mut accesskit_android::jni::JNIEnv<'_>,
        &accesskit_android::jni::objects::JObject<'_>,
    ) -> accesskit_android::jni::errors::Result<T>,
) -> Option<T> {
    use std::mem::ManuallyDrop;

    use accesskit_android::jni::{self, objects::JObject};

    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    // android_main never returns to Java, so only a frame frees these locals.
    let result = env.with_local_frame(16, |env| {
        // `activity_as_ptr` is a global ref; never drop it as a local ref.
        let activity = ManuallyDrop::new(unsafe {
            JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject)
        });
        read(env, &activity)
    });
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            let _ = env.exception_clear();
            log::warn!("nana-android-host: {what}: {error}");
            None
        }
    }
}

/// System bars, display cutout and the soft keyboard on all four edges, in
/// physical px, from the decor view's root insets. Before API 30 the system
/// window insets already include the keyboard under `adjustResize`.
fn safe_area(app: &AndroidApp) -> SafeArea {
    use accesskit_android::jni::objects::JValue;

    with_activity(app, "window insets", |env, activity| {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                "getRootWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(SafeArea::default());
        }
        let sdk = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;
        let edges = if sdk >= 30 {
            let types = "android/view/WindowInsets$Type";
            let bars = env
                .call_static_method(types, "systemBars", "()I", &[])?
                .i()?;
            let cutout = env
                .call_static_method(types, "displayCutout", "()I", &[])?
                .i()?;
            let ime = env.call_static_method(types, "ime", "()I", &[])?.i()?;
            let mut rect = |mask: i32| -> accesskit_android::jni::errors::Result<[i32; 4]> {
                let rect = env
                    .call_method(
                        &insets,
                        "getInsets",
                        "(I)Landroid/graphics/Insets;",
                        &[JValue::Int(mask)],
                    )?
                    .l()?;
                let mut side =
                    |name: &str| env.get_field(&rect, name, "I").and_then(|value| value.i());
                Ok([side("top")?, side("right")?, side("bottom")?, side("left")?])
            };
            let system = rect(bars | cutout)?;
            let keyboard = rect(ime)?;
            [system[0], system[1], system[2].max(keyboard[2]), system[3]]
        } else {
            let mut side = |name: &str| {
                env.call_method(&insets, name, "()I", &[])
                    .and_then(|value| value.i())
            };
            [
                side("getSystemWindowInsetTop")?,
                side("getSystemWindowInsetRight")?,
                side("getSystemWindowInsetBottom")?,
                side("getSystemWindowInsetLeft")?,
            ]
        };
        let [top, right, bottom, left] = edges.map(|edge| edge.max(0) as f32);
        Ok(SafeArea {
            top,
            right,
            bottom,
            left,
        })
    })
    .unwrap_or_default()
}

/// The system text-size setting, `Resources.getConfiguration().fontScale`.
/// `AConfiguration` does not carry it.
fn font_scale(app: &AndroidApp) -> f32 {
    with_activity(app, "font scale", |env, activity| {
        let resources = env
            .call_method(
                activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;
        let configuration = env
            .call_method(
                &resources,
                "getConfiguration",
                "()Landroid/content/res/Configuration;",
                &[],
            )?
            .l()?;
        env.get_field(&configuration, "fontScale", "F")?.f()
    })
    .filter(|scale| scale.is_finite() && *scale > 0.0)
    .unwrap_or(1.0)
}
