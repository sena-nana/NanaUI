//! Default application mark, Windows PE embed, and macOS `.app` packaging.
//!
//! The geometric mark is a fallback identity. Applications register their own
//! [`nana_ui_platform::WindowIcon`] at runtime, or embed a custom `.ico` from `build.rs`.

mod encode;
mod mark;
mod package;

#[cfg(feature = "embed")]
mod embed;

pub use encode::png as encode_png;
pub use package::{MacAppPackage, package_macos_app};

#[cfg(feature = "embed")]
pub use embed::{embed_windows, embed_windows_from};

#[cfg(feature = "runtime")]
mod runtime_api {
    use nana_ui_platform::{WindowIcon, resolve_window_icon};

    use crate::mark::rasterize;

    pub fn window_icon_from_png(bytes: &[u8]) -> Result<WindowIcon, String> {
        let image = image::load_from_memory(bytes)
            .map_err(|error| error.to_string())?
            .to_rgba8();
        let (width, height) = (image.width(), image.height());
        WindowIcon::from_rgba(image.into_raw(), width, height).map_err(|error| error.to_string())
    }

    pub fn default_window_icon() -> WindowIcon {
        WindowIcon::from_rgba(rasterize(256), 256, 256).expect("default mark is valid RGBA")
    }

    pub fn resolved_application_icon(per_window: Option<&WindowIcon>) -> WindowIcon {
        resolve_window_icon(per_window).unwrap_or_else(default_window_icon)
    }
}

#[cfg(feature = "runtime")]
pub use runtime_api::{default_window_icon, resolved_application_icon, window_icon_from_png};

#[cfg(all(test, feature = "runtime"))]
mod tests {
    use nana_ui_platform::{
        WindowIcon, clear_registered_application_icon, register_application_icon,
    };

    use super::*;

    #[test]
    fn default_icon_is_256_rgba() {
        let icon = default_window_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(icon.rgba.iter().any(|channel| *channel > 0));
    }

    #[test]
    fn resolved_icon_prefers_window_then_registered_then_default() {
        clear_registered_application_icon();
        let registered = WindowIcon::from_rgba(vec![1, 2, 3, 4], 1, 1).unwrap();
        register_application_icon(registered.clone());
        assert_eq!(resolved_application_icon(None), registered);
        let overlay = WindowIcon::from_rgba(vec![9, 8, 7, 6], 1, 1).unwrap();
        assert_eq!(resolved_application_icon(Some(&overlay)), overlay);
        clear_registered_application_icon();
        assert_eq!(resolved_application_icon(None).width, 256);
    }
}
