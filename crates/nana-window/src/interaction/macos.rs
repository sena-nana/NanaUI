use std::sync::Arc;

use nana_ui_platform::{
    InputRegionOutcome, InputRegionSnapshot, MousePassthroughMode, NativeHitTestClass,
    WindowInteractionCapabilities,
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
        snapshot: &Arc<InputRegionSnapshot>,
        mode: MousePassthroughMode,
        _scale_factor: f32,
    ) -> InputRegionOutcome {
        let rect_count = snapshot
            .rects
            .iter()
            .filter(|r| r.class != NativeHitTestClass::PassThrough)
            .count();
        match mode {
            MousePassthroughMode::Off => InputRegionOutcome::WholeWindow { passthrough: false },
            MousePassthroughMode::Passthrough => {
                InputRegionOutcome::WholeWindow { passthrough: true }
            }
            // Public AppKit exposes no non-SPI per-rect cross-process input shape API;
            // macOS routes #49 `Forward` through `InputRegionSnapshot::classify`.
            MousePassthroughMode::Forward => InputRegionOutcome::ForwardFallback { rect_count },
        }
    }

    pub(super) fn release(&mut self) {}
}

pub(super) fn probe_capabilities<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    _window: &W,
) -> WindowInteractionCapabilities {
    WindowInteractionCapabilities {
        input_region: false,
        non_client_chrome: false,
        passthrough_forward: true,
    }
}
