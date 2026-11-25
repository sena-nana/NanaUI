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
//!
//! What the window shows is an [`AndroidHostApp`]: the probe page in this
//! crate's own APK, or an app crate's view through [`crate::export_android_app!`].

use std::any::Any;
use std::time::{Duration, Instant};

use android_activity::input::{
    ImeOptions, InputEvent, InputType, KeyAction, Keycode, MetaState, MotionAction,
    TextInputAction, TextInputState, TextSpan,
};
use android_activity::ndk::configuration::UiModeNight;
use android_activity::{AndroidApp, AndroidAppWaker, InputStatus, MainEvent, PollEvent};
use nana_mobile_host::{
    FrameDemand, MobileAppearance, MobilePlatform, MobileRenderer, MobileRuntime, MobileViewport,
    SafeArea, ScrollPhysics, SurfaceOptions, TouchPhase,
};
use nana_ui::runtime::FrameworkError;
use nana_ui_platform::{SharedClipboardHost, TextInputContext, default_shared_clipboard};
use raw_window_handle::{AndroidNdkWindowHandle, DisplayHandle, RawWindowHandle, WindowHandle};

use crate::choreographer::ChoreographerDriver;
use crate::shell::{SystemInsets, primary_rect, scale_factor_from_density_dpi};
use crate::slot_ax::SlotAccessibility;
use crate::slot_ime::{
    SlotCursorAnchorInfo, SlotEditorInfo, SlotImeBuffer, build_cursor_anchor_info,
    ime_events_from_buffer_delta,
};
use crate::slot_input::{
    SlotInputGate, SlotKeyDispatch, SlotKeyMods, SlotLogicalKey, SlotTouchKind,
    android_keycode_is_modifier, host_swallows_for_input_connection,
    logical_key_from_android_keycode, slot_key_to_dispatch,
};

/// Whether the activity is in front: an app pauses what only the visible
/// window needs (a camera, a discovery socket) while it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Resumed,
    Paused,
}

/// Feedback played through the window's `View.performHapticFeedback`, the
/// same five styles as the iOS host's `Haptic`.
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

impl Haptic {
    /// The `HapticFeedbackConstants` value on this SDK level: `CONFIRM` and
    /// `REJECT` exist from API 30.
    fn constant(self, sdk: i32) -> i32 {
        const LONG_PRESS: i32 = 0;
        const VIRTUAL_KEY: i32 = 1;
        const CLOCK_TICK: i32 = 4;
        const CONTEXT_CLICK: i32 = 6;
        const CONFIRM: i32 = 16;
        const REJECT: i32 = 17;
        match self {
            Self::Light => VIRTUAL_KEY,
            Self::Medium => CONTEXT_CLICK,
            Self::Selection => CLOCK_TICK,
            Self::Success if sdk >= 30 => CONFIRM,
            Self::Success => VIRTUAL_KEY,
            Self::Warning if sdk >= 30 => REJECT,
            Self::Warning => LONG_PRESS,
        }
    }
}

/// What an app reaches the host through, from any thread.
#[derive(Clone)]
pub struct Shell {
    app: AndroidApp,
    waker: AndroidAppWaker,
}

impl Shell {
    /// Ask the loop for a pass: state the document reads changed off the
    /// loop's thread.
    pub fn wake(&self) {
        self.waker.wake();
    }

    /// Play `style` on the window's decor view.
    pub fn haptic(&self, style: Haptic) {
        use accesskit_android::jni::objects::JValue;

        with_activity(&self.app, "haptic", |env, activity| {
            let sdk = env
                .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
                .i()?;
            let window = env
                .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
                .l()?;
            let decor = env
                .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?;
            env.call_method(
                &decor,
                "performHapticFeedback",
                "(I)Z",
                &[JValue::Int(style.constant(sdk))],
            )?;
            Ok(())
        });
    }

    /// The activity, for an app's own JNI calls into its Java side.
    pub fn android_app(&self) -> &AndroidApp {
        &self.app
    }
}

/// The application the window shows: it mounts its view once, when the
/// first window arrives, and may prepare host textures or other per-frame
/// state before each pass. The loop's thread only.
pub trait AndroidHostApp: Any {
    fn mount(&mut self, runtime: &mut MobileRuntime, shell: Shell) -> Result<(), FrameworkError>;

    /// Runs on every pass of the loop before the frame demand is resolved.
    fn before_frame(&mut self, _runtime: &mut MobileRuntime, _renderer: &mut MobileRenderer) {}

    /// A pass the app needs besides the document's (a deadline of its own).
    fn demand(&self) -> FrameDemand {
        FrameDemand::Idle
    }

    fn lifecycle(&mut self, _runtime: &mut MobileRuntime, _event: Lifecycle) {}
}

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
    ax: Option<SlotAccessibility>,
}

struct Host {
    app: Box<dyn AndroidHostApp>,
    shell: Shell,
    /// The app is mounted once, into the first document.
    mounted: bool,
    /// Between `Resume` and `Pause`.
    foreground: bool,
    document: Option<Document>,
    gate: SlotInputGate,
    insets: SystemInsets,
    ime_shown: bool,
    ime_buffer: SlotImeBuffer,
    editor_info: Option<SlotEditorInfo>,
    cursor_anchor: Option<SlotCursorAnchorInfo>,
}

impl Host {
    fn new(app: Box<dyn AndroidHostApp>, shell: Shell) -> Self {
        Self {
            app,
            shell,
            mounted: false,
            foreground: false,
            document: None,
            gate: SlotInputGate::default(),
            insets: SystemInsets::default(),
            ime_shown: false,
            ime_buffer: SlotImeBuffer::default(),
            editor_info: None,
            cursor_anchor: None,
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
        if !self.mounted {
            self.app
                .mount(&mut runtime, self.shell.clone())
                .map_err(|error| format!("mount: {error}"))?;
            self.mounted = true;
        }
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
            ax,
        });
        // `Resume` came before the first window.
        if self.foreground {
            self.lifecycle(Lifecycle::Resumed);
        }
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
        self.cursor_anchor = None;
        log::info!("nana-android-host: surface destroyed, document kept");
    }

    /// When the loop should come back without input.
    fn demand(&self) -> FrameDemand {
        let Some(document) = self.document.as_ref().filter(|_| self.attached()) else {
            return FrameDemand::Idle;
        };
        let mut demand = document.runtime.frame_demand().merge(self.app.demand());
        if document.renderer.host_textures_changed() {
            demand = demand.merge(FrameDemand::Now);
        }
        demand
    }

    /// One pass's frame, when it is due.
    fn frame(&mut self, now: Instant) {
        if !self.attached() {
            return;
        }
        if let Some(document) = self.document.as_mut() {
            self.app
                .before_frame(&mut document.runtime, &mut document.renderer);
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

    fn lifecycle(&mut self, event: Lifecycle) {
        self.foreground = event == Lifecycle::Resumed;
        if let Some(document) = self.document.as_mut() {
            self.app.lifecycle(&mut document.runtime, event);
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
            if let Some(anchor) = self.build_current_cursor_anchor()
                && self.cursor_anchor.as_ref() != Some(&anchor)
            {
                update_cursor_anchor_info(app, &anchor);
                self.cursor_anchor = Some(anchor);
            }
        } else {
            self.editor_info = None;
            self.cursor_anchor = None;
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
                self.cursor_anchor = None;
            }
            _ => {}
        }
    }

    fn build_current_cursor_anchor(&mut self) -> Option<SlotCursorAnchorInfo> {
        let document = self.document.as_mut()?;
        let snapshot = document.runtime.text_input()?;
        let bounds = document.runtime.text_input_bounds()?;
        let caret = document
            .runtime
            .text_caret_rect(snapshot.selection.1)
            .unwrap_or(bounds);
        let scale = document.runtime.viewport().scale;
        Some(build_cursor_anchor_info(
            &snapshot.text,
            snapshot.selection,
            snapshot.marked,
            (caret.x, caret.y, caret.width, caret.height),
            (bounds.x, bounds.y, bounds.width, bounds.height),
            scale,
        ))
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

/// Run the activity's loop around `host_app` until `Destroy`; the body of
/// [`crate::export_android_app!`]'s `android_main`.
pub fn run(app: AndroidApp, host_app: Box<dyn AndroidHostApp>) -> Result<(), String> {
    let waker = app.create_waker();
    let task_waker = waker.clone();
    nana_mobile_host::set_task_wake(move || task_waker.wake());
    let choreographer = ChoreographerDriver::new(waker.clone());
    let shell = Shell {
        app: app.clone(),
        waker,
    };
    let mut host = Host::new(host_app, shell);
    let mut running = true;

    while running {
        let now = Instant::now();
        let timeout = match host.demand().resolve(now) {
            FrameDemand::Now if choreographer.is_frame_ready() => Some(Duration::ZERO),
            FrameDemand::Now => {
                choreographer.request_vsync();
                // Bounded guard in case an Activity stops delivering choreographer ticks.
                Some(Duration::from_millis(50))
            }
            FrameDemand::At(deadline) => Some(deadline.saturating_duration_since(now)),
            FrameDemand::Idle => {
                choreographer.disarm();
                None
            }
        };
        app.poll_events(timeout, |event| {
            let PollEvent::Main(main) = event else {
                return;
            };
            match main {
                MainEvent::InitWindow { .. } => {
                    if let Err(error) = host.on_window_ready(&app) {
                        log::error!("nana-android-host: window ready failed: {error}");
                    } else {
                        choreographer.mark_immediate_ready();
                    }
                }
                MainEvent::TerminateWindow { .. } => {
                    choreographer.disarm();
                    host.on_window_destroyed();
                }
                MainEvent::Destroy => {
                    choreographer.disarm();
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
                    if matches!(main, MainEvent::RedrawNeeded { .. }) {
                        choreographer.mark_immediate_ready();
                    }
                }
                MainEvent::Resume { .. } => host.lifecycle(Lifecycle::Resumed),
                MainEvent::Pause => {
                    // Android dismisses the IME itself; the next focus
                    // re-shows it.
                    host.ime_shown = false;
                    choreographer.disarm();
                    host.lifecycle(Lifecycle::Paused);
                }
                MainEvent::Stop => {
                    host.ime_shown = false;
                    choreographer.disarm();
                }
                _ => {}
            }
        });

        if let Ok(mut iter) = app.input_events_iter() {
            while iter.next(|event| {
                let handled = match event {
                    InputEvent::MotionEvent(motion) => {
                        let action = motion.action();
                        let handled = match action {
                            MotionAction::Move | MotionAction::Cancel => {
                                let mut any = false;
                                for pointer in motion.pointers() {
                                    any |= host.handle_motion(
                                        action,
                                        pointer.x(),
                                        pointer.y(),
                                        pointer.pointer_id(),
                                    );
                                }
                                any
                            }
                            _ => {
                                let pointer = motion.pointer_at_index(motion.pointer_index());
                                host.handle_motion(
                                    action,
                                    pointer.x(),
                                    pointer.y(),
                                    pointer.pointer_id(),
                                )
                            }
                        };
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
        let after_input = Instant::now();
        let post_demand = host.demand().resolve(after_input);
        if post_demand == FrameDemand::Now {
            if choreographer.take_frame_ready() {
                host.frame(after_input);
            } else {
                choreographer.request_vsync();
            }
        } else {
            choreographer.disarm();
        }
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

fn update_cursor_anchor_info(app: &AndroidApp, anchor: &SlotCursorAnchorInfo) {
    use accesskit_android::jni::objects::{JObject, JValue};

    with_activity(app, "updateCursorAnchorInfo", |env, activity| {
        let imm_service = env.new_string("input_method")?;
        let imm = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&imm_service)],
            )?
            .l()?;
        if imm.is_null() {
            return Ok(());
        }
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let current_focus = env
            .call_method(&window, "getCurrentFocus", "()Landroid/view/View;", &[])?
            .l()?;
        let view = if !current_focus.is_null() {
            current_focus
        } else {
            env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?
        };
        if view.is_null() {
            return Ok(());
        }

        let loc_arr = env.new_int_array(2)?;
        let loc_obj: JObject = loc_arr.into();
        env.call_method(
            &view,
            "getLocationInWindow",
            "([I)V",
            &[JValue::Object(&loc_obj)],
        )?;
        let mut loc = [0i32; 2];
        let loc_arr_ref: &accesskit_android::jni::objects::JIntArray = (&loc_obj).into();
        env.get_int_array_region(loc_arr_ref, 0, &mut loc)?;

        let matrix = env.new_object("android/graphics/Matrix", "()V", &[])?;
        env.call_method(
            &matrix,
            "setTranslate",
            "(FF)V",
            &[JValue::Float(loc[0] as f32), JValue::Float(loc[1] as f32)],
        )?;

        let builder = env.new_object(
            "android/view/inputmethod/CursorAnchorInfo$Builder",
            "()V",
            &[],
        )?;
        env.call_method(
            &builder,
            "setMatrix",
            "(Landroid/graphics/Matrix;)Landroid/view/inputmethod/CursorAnchorInfo$Builder;",
            &[JValue::Object(&matrix)],
        )?;
        env.call_method(
            &builder,
            "setSelectionRange",
            "(II)Landroid/view/inputmethod/CursorAnchorInfo$Builder;",
            &[
                JValue::Int(anchor.selection_start),
                JValue::Int(anchor.selection_end),
            ],
        )?;
        if let (Some(start), Some(text)) =
            (anchor.composing_start, anchor.composing_text.as_deref())
        {
            let jtext = env.new_string(text)?;
            env.call_method(
                &builder,
                "setComposingText",
                "(ILjava/lang/CharSequence;)Landroid/view/inputmethod/CursorAnchorInfo$Builder;",
                &[JValue::Int(start), JValue::Object(&jtext)],
            )?;
        }
        const FLAG_HAS_VISIBLE_REGION: i32 = 1;
        env.call_method(
            &builder,
            "setInsertionMarkerLocation",
            "(FFFFI)Landroid/view/inputmethod/CursorAnchorInfo$Builder;",
            &[
                JValue::Float(anchor.insertion_marker_x),
                JValue::Float(anchor.insertion_marker_y),
                JValue::Float(anchor.insertion_marker_bottom),
                JValue::Float(anchor.insertion_marker_bottom),
                JValue::Int(FLAG_HAS_VISIBLE_REGION),
            ],
        )?;
        let info = env
            .call_method(
                &builder,
                "build",
                "()Landroid/view/inputmethod/CursorAnchorInfo;",
                &[],
            )?
            .l()?;
        env.call_method(
            &imm,
            "updateCursorAnchorInfo",
            "(Landroid/view/View;Landroid/view/inputmethod/CursorAnchorInfo;)V",
            &[JValue::Object(&view), JValue::Object(&info)],
        )?;
        Ok(())
    });
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
