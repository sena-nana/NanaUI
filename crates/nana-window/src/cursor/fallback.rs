use raw_window_handle::HasWindowHandle;

use super::{CursorBitmap, CustomCursorCapabilities, CustomCursorOutcome};

#[derive(Clone)]
pub(super) struct FallbackCursorHandle {
    _private: (),
}

pub(super) fn capabilities() -> CustomCursorCapabilities {
    #[cfg(target_os = "linux")]
    {
        CustomCursorCapabilities {
            supported: true,
            multi_dpi: false,
            max_edge: 256,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        CustomCursorCapabilities {
            supported: false,
            multi_dpi: false,
            max_edge: 0,
        }
    }
}

pub(super) fn create_cursor(_bitmap: &CursorBitmap) -> Result<FallbackCursorHandle, String> {
    Ok(FallbackCursorHandle { _private: () })
}

pub(super) fn apply_custom_cursor<W: HasWindowHandle + ?Sized>(
    _window: &W,
    _handle: &FallbackCursorHandle,
    _scale_milli: u32,
) -> CustomCursorOutcome {
    CustomCursorOutcome::Unsupported(
        "native custom cursor handle application requires winit or a desktop backend".into(),
    )
}
