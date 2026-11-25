use std::sync::Arc;

use nana_ui_platform::{
    InputRegionOutcome, InputRegionSnapshot, MousePassthroughMode, WindowInteractionCapabilities,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[derive(Debug, Default)]
pub(super) struct BackendState;

impl BackendState {
    pub(super) fn set_pointer_captured<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        _captured: bool,
    ) {
    }

    pub(super) fn apply<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        _window: &W,
        _caps: WindowInteractionCapabilities,
        _snapshot: &Arc<InputRegionSnapshot>,
        mode: MousePassthroughMode,
        _scale_factor: f32,
    ) -> InputRegionOutcome {
        match mode {
            MousePassthroughMode::Passthrough => {
                InputRegionOutcome::WholeWindow { passthrough: true }
            }
            _ => InputRegionOutcome::Unsupported,
        }
    }

    pub(super) fn release(&mut self) {}
}

pub(super) fn probe_capabilities<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    _window: &W,
) -> WindowInteractionCapabilities {
    WindowInteractionCapabilities::default()
}
