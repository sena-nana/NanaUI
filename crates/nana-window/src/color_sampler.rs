use std::fmt;

use raw_window_handle::HasWindowHandle;

/// Whether the current desktop session provides native screen color sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSamplerSupport {
    /// A platform color sampler can read pixels anywhere on the desktop.
    Native,
    /// This platform or session does not expose a native screen color sampler.
    Unsupported,
}

impl ColorSamplerSupport {
    /// Returns `true` when [`pick_screen_color`] can start a native sampler.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Native)
    }
}

/// Returns whether screen color picking is available in the current session.
#[must_use]
pub fn color_sampler_support() -> ColorSamplerSupport {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        ColorSamplerSupport::Native
    }
    #[cfg(target_os = "linux")]
    {
        crate::color_sampler_linux::support()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        ColorSamplerSupport::Unsupported
    }
}

/// Why a screen color sampling session failed to start or finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorSamplerError {
    /// The platform or current display session has no native color sampler.
    Unsupported,
    /// The caller-supplied owner window did not expose a valid native handle.
    InvalidWindow,
    /// A native platform API reported a failure.
    Platform(String),
}

impl fmt::Display for ColorSamplerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("screen color sampling is unsupported"),
            Self::InvalidWindow => f.write_str("invalid owner window for screen color sampling"),
            Self::Platform(detail) => write!(f, "screen color sampling failed: {detail}"),
        }
    }
}

impl std::error::Error for ColorSamplerError {}

/// Outcome delivered once when a native screen color sampling session ends.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorSamplerResult {
    /// The user sampled a pixel, normalized into sRGB `[r, g, b, 1.0]`.
    Picked { request_id: u64, rgba: [f32; 4] },
    /// The user or caller cancelled the sampling session.
    Cancelled { request_id: u64 },
    /// The platform sampler aborted with an error.
    Failed {
        request_id: u64,
        error: ColorSamplerError,
    },
}

impl ColorSamplerResult {
    /// Builds a picked result from finite or out-of-range sRGB components,
    /// clamping each channel into `0.0..=1.0` and setting alpha to `1.0`.
    #[must_use]
    pub fn picked(request_id: u64, rgb: [f32; 3]) -> Self {
        let clamp = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        Self::Picked {
            request_id,
            rgba: [clamp(rgb[0]), clamp(rgb[1]), clamp(rgb[2]), 1.0],
        }
    }

    /// Builds a cancelled result for `request_id`.
    #[must_use]
    pub const fn cancelled(request_id: u64) -> Self {
        Self::Cancelled { request_id }
    }

    /// Builds a failed result for `request_id`.
    #[must_use]
    pub const fn failed(request_id: u64, error: ColorSamplerError) -> Self {
        Self::Failed { request_id, error }
    }

    /// Returns the request id associated with this result.
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Picked { request_id, .. }
            | Self::Cancelled { request_id }
            | Self::Failed { request_id, .. } => *request_id,
        }
    }

    /// Returns `true` if the session ended via cancellation.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
    }
}

/// Cancels an active native screen color sampler when dropped or cancelled
/// explicitly.
pub struct ColorSamplerHandle {
    cancel: Option<Box<dyn FnOnce() + Send>>,
}

impl fmt::Debug for ColorSamplerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorSamplerHandle")
            .field("active", &self.cancel.is_some())
            .finish()
    }
}

impl ColorSamplerHandle {
    #[allow(dead_code)]
    pub(crate) fn new(cancel: impl FnOnce() + Send + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }

    /// Requests cancellation of the in-flight sampler at most once.
    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

impl Drop for ColorSamplerHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Starts a native screen color sampler without blocking the caller, invoking
/// `completion` at most once when a color is picked, cancelled, or failed.
///
/// On macOS this uses `NSColorSampler`; on Windows it presents a
/// capture-excluded full-virtual-screen overlay with a live magnifier loupe; on
/// Linux it uses the XDG Screenshot `PickColor` portal with an X11 root-grab
/// fallback.
pub fn pick_screen_color<F>(
    window: Option<&dyn HasWindowHandle>,
    request_id: u64,
    completion: F,
) -> Result<ColorSamplerHandle, ColorSamplerError>
where
    F: FnOnce(ColorSamplerResult) + Send + 'static,
{
    if !color_sampler_support().is_supported() {
        return Err(ColorSamplerError::Unsupported);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = window;
        crate::platform::pick_screen_color(request_id, completion)
    }

    #[cfg(target_os = "windows")]
    {
        crate::color_sampler_windows::pick_screen_color(window, request_id, completion)
    }

    #[cfg(target_os = "linux")]
    {
        crate::color_sampler_linux::pick_screen_color(window, request_id, completion)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (window, request_id, completion);
        Err(ColorSamplerError::Unsupported)
    }
}

#[allow(dead_code)]
#[inline]
pub(crate) fn srgb_u8_to_rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    ]
}

#[allow(dead_code)]
#[inline]
pub(crate) fn srgb_u16_to_rgb(r: u16, g: u16, b: u16) -> [f32; 3] {
    [
        f32::from(r) / 65535.0,
        f32::from(g) / 65535.0,
        f32::from(b) / 65535.0,
    ]
}

#[allow(dead_code)]
#[inline]
pub(crate) fn hex_rgb_u8(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn srgb_conversions_clamp_and_preserve_opaque_alpha() {
        assert_eq!(srgb_u8_to_rgb(255, 0, 128)[0], 1.0);
        assert_eq!(srgb_u8_to_rgb(255, 0, 128)[1], 0.0);
        assert_eq!(hex_rgb_u8(0x1a, 0x2b, 0xff), "#1A2BFF");
        assert_eq!(srgb_u16_to_rgb(65535, 0, 65535), [1.0, 0.0, 1.0]);

        let picked = ColorSamplerResult::picked(9, [-0.5, 1.5, f32::NAN]);
        assert_eq!(
            picked,
            ColorSamplerResult::Picked {
                request_id: 9,
                rgba: [0.0, 1.0, 0.0, 1.0],
            }
        );
        assert_eq!(picked.request_id(), 9);
        assert!(!picked.is_cancelled());
        assert!(ColorSamplerResult::cancelled(3).is_cancelled());
    }

    #[test]
    fn handle_cancel_runs_once_and_drops_cleanly() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_cloned = Arc::clone(&count);
        let mut handle = ColorSamplerHandle::new(move || {
            count_cloned.fetch_add(1, Ordering::SeqCst);
        });
        handle.cancel();
        handle.cancel();
        drop(handle);
        assert_eq!(count.load(Ordering::SeqCst), 1);

        let drop_count = Arc::new(AtomicUsize::new(0));
        let drop_cloned = Arc::clone(&drop_count);
        {
            let _scoped = ColorSamplerHandle::new(move || {
                drop_cloned.fetch_add(1, Ordering::SeqCst);
            });
        }
        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn support_reflects_platform_session() {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(color_sampler_support(), ColorSamplerSupport::Native);

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        assert_eq!(color_sampler_support(), ColorSamplerSupport::Unsupported);
    }
}
