use std::cell::{Cell, RefCell};
use std::sync::Arc;

use nana_ui_platform::{
    InputRegionOutcome, InputRegionSnapshot, MousePassthroughMode, WindowInteractionCapabilities,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use windows_sys::Win32::Foundation::HWND;

use crate::win32_hooks::Win32WindowHooks;

#[derive(Clone)]
pub(crate) struct Win32InteractionHookState {
    inner: Arc<SharedHookState>,
}

struct SharedHookState {
    snapshot: std::sync::Mutex<Option<Arc<InputRegionSnapshot>>>,
    mode: std::sync::atomic::AtomicU8,
    pointer_captured: std::sync::atomic::AtomicBool,
    scale_milli: std::sync::atomic::AtomicU32,
}

impl std::fmt::Debug for Win32InteractionHookState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Win32InteractionHookState").finish()
    }
}

impl Win32InteractionHookState {
    fn new() -> Self {
        Self {
            inner: Arc::new(SharedHookState {
                snapshot: std::sync::Mutex::new(None),
                mode: std::sync::atomic::AtomicU8::new(0),
                pointer_captured: std::sync::atomic::AtomicBool::new(false),
                scale_milli: std::sync::atomic::AtomicU32::new(1000),
            }),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Arc<InputRegionSnapshot>> {
        self.inner
            .snapshot
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) fn mode(&self) -> MousePassthroughMode {
        match self.inner.mode.load(std::sync::atomic::Ordering::Relaxed) {
            1 => MousePassthroughMode::Passthrough,
            2 => MousePassthroughMode::Forward,
            _ => MousePassthroughMode::Off,
        }
    }

    pub(crate) fn pointer_captured(&self) -> bool {
        self.inner
            .pointer_captured
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn scale_factor(&self) -> f32 {
        let milli = self
            .inner
            .scale_milli
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(1);
        f32::from(milli as u16) / 1000.0
    }
}

#[derive(Debug, Default)]
pub(super) struct BackendState {
    hook: Option<Win32InteractionHookState>,
    hwnd: Option<isize>,
}

impl BackendState {
    fn ensure_hook<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
    ) -> Option<Win32InteractionHookState> {
        if let Some(hook) = &self.hook {
            return Some(hook.clone());
        }
        let hwnd = crate::win32::hwnd(window)?;
        let hooks = Win32WindowHooks::install_or_get(hwnd).ok()?;
        let state = Win32InteractionHookState::new();
        *hooks.interaction.borrow_mut() = Some(state.clone());
        self.hwnd = Some(hwnd as isize);
        self.hook = Some(state.clone());
        Some(state)
    }

    pub(super) fn set_pointer_captured<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
        captured: bool,
    ) {
        if let Some(hook) = self.ensure_hook(window) {
            hook.inner
                .pointer_captured
                .store(captured, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(super) fn apply<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        window: &W,
        _caps: WindowInteractionCapabilities,
        snapshot: &Arc<InputRegionSnapshot>,
        mode: MousePassthroughMode,
        scale_factor: f32,
    ) -> InputRegionOutcome {
        let Some(hook) = self.ensure_hook(window) else {
            return InputRegionOutcome::Unsupported;
        };
        if let Ok(mut guard) = hook.inner.snapshot.lock() {
            *guard = Some(Arc::clone(snapshot));
        }
        let mode_code = match mode {
            MousePassthroughMode::Off => 0,
            MousePassthroughMode::Passthrough => 1,
            MousePassthroughMode::Forward => 2,
        };
        hook.inner
            .mode
            .store(mode_code, std::sync::atomic::Ordering::Relaxed);
        let scale_milli = if scale_factor.is_finite() && scale_factor > 0.0 {
            (scale_factor * 1000.0).round().clamp(250.0, 16000.0) as u32
        } else {
            1000
        };
        hook.inner
            .scale_milli
            .store(scale_milli, std::sync::atomic::Ordering::Relaxed);

        let rect_count = snapshot
            .rects
            .iter()
            .filter(|r| r.class != nana_ui_platform::NativeHitTestClass::PassThrough)
            .count();
        match mode {
            MousePassthroughMode::Passthrough => {
                InputRegionOutcome::WholeWindow { passthrough: true }
            }
            // Forward on transparent DComp HWNDs still uses OS WS_EX_TRANSPARENT toggling
            // (unified with InputRegionSnapshot::classify) for cross-process passthrough,
            // while WM_NCHITTEST enforces HTCLIENT/HTCAPTION/HT* whenever active.
            MousePassthroughMode::Forward => InputRegionOutcome::ForwardFallback { rect_count },
            MousePassthroughMode::Off => InputRegionOutcome::NativeApplied { rect_count },
        }
    }

    pub(super) fn release(&mut self) {
        if let Some(hwnd_raw) = self.hwnd.take()
            && let Some(hooks) = Win32WindowHooks::get(hwnd_raw as HWND)
        {
            *hooks.interaction.borrow_mut() = None;
        }
        self.hook = None;
    }
}

pub(super) fn probe_capabilities<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    window: &W,
) -> WindowInteractionCapabilities {
    let has_hwnd = crate::win32::hwnd(window).is_some();
    WindowInteractionCapabilities {
        input_region: has_hwnd,
        non_client_chrome: has_hwnd,
        passthrough_forward: has_hwnd,
    }
}
