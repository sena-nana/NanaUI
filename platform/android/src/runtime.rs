//! Android activity loop: window lifecycle → wgpu Surface → QuickJS shell stub.

use std::time::{Duration, Instant};

use android_activity::input::{InputEvent, KeyAction, Keycode, MetaState, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use nana_ui_platform::SurfacePhase;
use nana_ui_vue::VueHost;

use crate::engine::smoke_engine_only;
use crate::gpu::GpuSurface;
use crate::iced_slot_input::{
    SlotKeyMods, SlotTouchKind, android_keycode_is_modifier, logical_key_from_android_keycode,
};
use crate::iced_slot_paint::IcedSlotPainter;
use crate::shell::{AndroidShellStub, scale_factor_from_density_dpi};

struct HostState {
    gpu: Option<GpuSurface>,
    iced_slot: Option<IcedSlotPainter>,
    vue: Option<VueHost>,
    shell: AndroidShellStub,
    engine_booted: bool,
    last_paint: Instant,
    phase: SurfacePhase,
}

impl HostState {
    fn new() -> Self {
        Self {
            gpu: None,
            iced_slot: None,
            vue: None,
            shell: AndroidShellStub::new(),
            engine_booted: false,
            last_paint: Instant::now() - Duration::from_secs(1),
            phase: SurfacePhase::Pending,
        }
    }

    fn ensure_engine(&mut self) {
        if self.engine_booted {
            return;
        }
        match smoke_engine_only() {
            Ok(report) => {
                log::info!(
                    "nana-android-host: QuickJS ok={} count={} createElement={} caps={:?}",
                    report.ok,
                    report.count,
                    report.create_element,
                    report.capabilities
                );
                self.engine_booted = true;
                let (pw, ph) = self.shell.primary_physical_size();
                self.vue = Some(VueHost::with_viewport(pw, ph, self.shell.scale_factor()));
            }
            Err(err) => log::error!("nana-android-host: engine boot failed: {err}"),
        }
    }

    fn scale_from_app(app: &AndroidApp) -> f32 {
        let density = app.config().density();
        let scale = scale_factor_from_density_dpi(density);
        log::debug!("nana-android-host: density_dpi={density:?} scale_factor={scale}");
        scale
    }

    fn window_physical_size(app: &AndroidApp) -> (u32, u32) {
        app.native_window()
            .map(|win| (win.width().max(1) as u32, win.height().max(1) as u32))
            .unwrap_or((720, 1280))
    }

    fn apply_scale_and_size(&mut self, physical: (u32, u32), scale: f32) {
        let (w, h) = physical;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(w, h);
        }
        self.shell.resize(w, h, scale);
        if let Some(painter) = self.iced_slot.as_mut() {
            painter.resize((w, h), scale);
        }
    }

    fn on_window_ready(&mut self, app: &AndroidApp) -> Result<(), String> {
        let (w, h) = Self::window_physical_size(app);
        let scale = Self::scale_from_app(app);

        if self.gpu.is_some() {
            self.apply_scale_and_size((w, h), scale);
            self.phase = SurfacePhase::Ready;
            return Ok(());
        }

        let gpu = GpuSurface::new(app, w, h)?;
        self.iced_slot = Some(IcedSlotPainter::new(
            &gpu.adapter,
            &gpu.device,
            &gpu.queue,
            gpu.format,
            (gpu.config.width, gpu.config.height),
            scale,
        ));
        self.gpu = Some(gpu);
        self.shell.resize(w, h, scale);
        self.phase = SurfacePhase::Ready;
        self.ensure_engine();
        log::info!(
            "nana-android-host: iced slot strip ready (Text+Input+Switch+Button, not DesktopShell) iced_shell={} scale={scale}",
            AndroidShellStub::iced_shell_available()
        );
        self.paint_frame()?;
        Ok(())
    }

    /// Density / orientation config change — refresh scale then resize geometry.
    fn on_config_changed(&mut self, app: &AndroidApp) {
        let scale = Self::scale_from_app(app);
        let physical = app
            .native_window()
            .map(|win| (win.width().max(1) as u32, win.height().max(1) as u32))
            .unwrap_or_else(|| self.shell.geometry().physical_size);
        self.apply_scale_and_size(physical, scale);
        log::info!("nana-android-host: ConfigChanged scale={scale} physical={physical:?}");
    }

    fn on_window_destroyed(&mut self) {
        self.iced_slot = None;
        self.gpu = None;
        self.phase = SurfacePhase::Destroyed;
        log::info!("nana-android-host: surface destroyed");
    }

    fn paint_frame(&mut self) -> Result<(), String> {
        let scale = self.shell.scale_factor();
        let (fw, fh, format) = {
            let Some(gpu) = self.gpu.as_ref() else {
                return Ok(());
            };
            (gpu.config.width, gpu.config.height, gpu.format)
        };
        self.shell.resize(fw, fh, scale);
        if let Some(painter) = self.iced_slot.as_mut() {
            painter.resize((fw, fh), scale);
        }

        if let Some(vue) = self.vue.as_mut() {
            let (pw, ph) = self.shell.primary_physical_size();
            vue.set_viewport(pw, ph, scale);
            vue.resolve_layout();
        }

        // Chrome scissor fill, then one Iced Button in the Primary control-slot
        // region (bottom-aligned). Not DesktopShell.
        let bands = self.shell.chrome_present_bands();
        log::trace!(
            "nana-android-host: chrome+iced bands={} iced_shell={} iced_slot_widget={}",
            bands.len(),
            AndroidShellStub::iced_shell_available(),
            AndroidShellStub::iced_control_widget_available()
        );
        let HostState {
            gpu,
            iced_slot,
            last_paint,
            ..
        } = self;
        let Some(gpu) = gpu.as_ref() else {
            return Ok(());
        };
        gpu.present_chrome_bands_with_overlay(&bands, |view| {
            if let Some(painter) = iced_slot.as_mut() {
                painter.paint_slot_button(view, format);
            }
            Ok(())
        })?;
        *last_paint = Instant::now();
        Ok(())
    }

    fn handle_motion(
        &mut self,
        action: MotionAction,
        physical_x: f32,
        physical_y: f32,
        pointer_id: i32,
    ) -> bool {
        let kind = match action {
            MotionAction::Down | MotionAction::PointerDown => SlotTouchKind::Down,
            MotionAction::Move | MotionAction::HoverMove => SlotTouchKind::Move,
            MotionAction::Up | MotionAction::PointerUp => SlotTouchKind::Up,
            MotionAction::Cancel => SlotTouchKind::Cancel,
            _ => return false,
        };
        let slot = self.shell.iced_control_slot().map(|b| b.rect);
        let Some(painter) = self.iced_slot.as_mut() else {
            return false;
        };
        // Only slot-local (or captured drag) samples are Handled; outside → VueHost.
        painter.push_touch(slot, kind, physical_x, physical_y, pointer_id)
    }

    /// NativeActivity KeyEvent → iced keyboard (US-QWERTY subset + editing keys).
    ///
    /// System keys (Back, …) stay `Unhandled`. Soft IME without KeyEvent is still open.
    /// Keys are Handled only while the iced slot holds keyboard focus (last Down
    /// was inside the slot); otherwise they remain available to VueHost.
    fn handle_key(
        &mut self,
        action: KeyAction,
        keycode: Keycode,
        meta: MetaState,
        repeat_count: i32,
    ) -> bool {
        let down = match action {
            KeyAction::Down => true,
            KeyAction::Up => false,
            // Treat ACTION_MULTIPLE as a single press for the printable subset.
            KeyAction::Multiple => true,
            _ => return false,
        };
        let keycode_u32: u32 = keycode.into();
        let mods = SlotKeyMods {
            shift: meta.shift_on(),
            ctrl: meta.ctrl_on(),
            alt: meta.alt_on(),
            logo: meta.meta_on(),
        };
        let logical =
            logical_key_from_android_keycode(keycode_u32, mods.shift, meta.caps_lock_on());
        let is_mod = android_keycode_is_modifier(keycode_u32);
        if logical.is_none() && !is_mod {
            return false;
        }
        let Some(painter) = self.iced_slot.as_mut() else {
            return false;
        };
        let repeat = down && repeat_count > 0;
        painter.push_key(down, logical, mods, repeat)
    }
}

pub fn run(app: AndroidApp) -> Result<(), String> {
    let mut state = HostState::new();
    state.ensure_engine();
    let mut running = true;

    while running {
        app.poll_events(Some(Duration::from_millis(16)), |event| match event {
            PollEvent::Main(main) => match main {
                MainEvent::InitWindow { .. } => {
                    if let Err(err) = state.on_window_ready(&app) {
                        log::error!("nana-android-host: window ready failed: {err}");
                    }
                }
                MainEvent::TerminateWindow { .. } => {
                    state.on_window_destroyed();
                }
                MainEvent::Destroy => {
                    state.on_window_destroyed();
                    running = false;
                }
                MainEvent::WindowResized { .. } | MainEvent::RedrawNeeded { .. } => {
                    if let Some(win) = app.native_window() {
                        let physical = (win.width().max(1) as u32, win.height().max(1) as u32);
                        // Keep current scale; ConfigChanged owns density updates.
                        state.apply_scale_and_size(physical, state.shell.scale_factor());
                    }
                    if let Err(err) = state.paint_frame() {
                        log::warn!("nana-android-host: paint: {err}");
                    }
                }
                MainEvent::ConfigChanged { .. } => {
                    state.on_config_changed(&app);
                    if let Err(err) = state.paint_frame() {
                        log::warn!("nana-android-host: config paint: {err}");
                    }
                }
                MainEvent::Pause | MainEvent::Stop => {
                    log::info!("nana-android-host: pause/stop phase={:?}", state.phase);
                }
                MainEvent::Resume { .. } | MainEvent::Start => {
                    log::info!("nana-android-host: resume/start");
                    state.ensure_engine();
                }
                _ => {}
            },
            PollEvent::Timeout => {
                if state.phase == SurfacePhase::Ready
                    && state.last_paint.elapsed() > Duration::from_millis(500)
                {
                    if let Err(err) = state.paint_frame() {
                        log::warn!("nana-android-host: heartbeat paint: {err}");
                    }
                }
            }
            _ => {}
        });

        let mut need_paint = false;
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let read = iter.next(|event| match event {
                    InputEvent::MotionEvent(motion) => {
                        let action = motion.action();
                        let idx = motion.pointer_index();
                        let pointer = motion.pointer_at_index(idx);
                        let x = pointer.x();
                        let y = pointer.y();
                        let pointer_id = pointer.pointer_id();
                        if state.handle_motion(action, x, y, pointer_id) {
                            need_paint = true;
                            InputStatus::Handled
                        } else {
                            InputStatus::Unhandled
                        }
                    }
                    InputEvent::KeyEvent(key) => {
                        if state.handle_key(
                            key.action(),
                            key.key_code(),
                            key.meta_state(),
                            key.repeat_count(),
                        ) {
                            need_paint = true;
                            InputStatus::Handled
                        } else {
                            InputStatus::Unhandled
                        }
                    }
                    _ => InputStatus::Unhandled,
                });
                if !read {
                    break;
                }
            }
        }
        if need_paint {
            if let Err(err) = state.paint_frame() {
                log::warn!("nana-android-host: input paint: {err}");
            }
        }
    }

    Ok(())
}
