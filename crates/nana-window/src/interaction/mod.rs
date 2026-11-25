//! Platform-owned native window input-region & non-client hit-test sync (#159).
//!
//! Applies [`InputRegionSnapshot`] to native OS backends without touching visual
//! clipping (`SetWindowRgn` / `WindowVisualShape`), and preserves active pointer
//! capture across updates.

use std::sync::Arc;

use nana_diagnostics::{metric, span};
use nana_ui_platform::{
    InputRegionOutcome, InputRegionSnapshot, MousePassthroughMode, WindowInteractionCapabilities,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;
#[cfg(target_os = "windows")]
pub(crate) use windows::Win32InteractionHookState;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
use fallback as imp;

/// Per-window retained native interaction state owned by the host.
#[derive(Debug)]
pub struct WindowInteractionState {
    capabilities: Option<WindowInteractionCapabilities>,
    outcome: InputRegionOutcome,
    last_revision: u64,
    last_mode: MousePassthroughMode,
    pointer_captured: bool,
    backend: imp::BackendState,
}

impl Default for WindowInteractionState {
    fn default() -> Self {
        Self {
            capabilities: None,
            outcome: InputRegionOutcome::WholeWindow { passthrough: false },
            last_revision: 0,
            last_mode: MousePassthroughMode::Off,
            pointer_captured: false,
            backend: imp::BackendState::default(),
        }
    }
}

impl Drop for WindowInteractionState {
    fn drop(&mut self) {
        self.backend.release();
    }
}

impl WindowInteractionState {
    /// Cached or lazily probed backend capabilities for `window`.
    pub fn capabilities<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        window: &W,
    ) -> WindowInteractionCapabilities {
        *self
            .capabilities
            .get_or_insert_with(|| imp::probe_capabilities(window))
    }

    pub fn cached_capabilities(&self) -> WindowInteractionCapabilities {
        self.capabilities.unwrap_or_default()
    }

    pub fn last_outcome(&self) -> &InputRegionOutcome {
        &self.outcome
    }

    /// Update active pointer-capture status so native hit-test hooks (`WM_NCHITTEST`)
    /// and OS input-region backends never interrupt a live capture/drag gesture.
    pub fn set_pointer_captured<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
        captured: bool,
    ) {
        if self.pointer_captured == captured {
            return;
        }
        self.pointer_captured = captured;
        self.backend.set_pointer_captured(window, captured);
    }

    /// Apply `snapshot` under `mode` to the native OS window backend.
    pub fn apply<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        window: &W,
        snapshot: &Arc<InputRegionSnapshot>,
        mode: MousePassthroughMode,
        scale_factor: f32,
    ) -> InputRegionOutcome {
        let caps = self.capabilities(window);
        if self.last_revision == snapshot.revision && self.last_mode == mode {
            return self.outcome.clone();
        }
        let _span = span!(nana_diagnostics::framework::window::INTERACTION_NATIVE_REGION_UPDATE_NS);
        metric!(nana_diagnostics::framework::window::INTERACTION_NATIVE_REGION_UPDATES);
        let outcome = self
            .backend
            .apply(window, caps, snapshot, mode, scale_factor);
        self.last_revision = snapshot.revision;
        self.last_mode = mode;
        self.outcome = outcome.clone();
        outcome
    }
}
