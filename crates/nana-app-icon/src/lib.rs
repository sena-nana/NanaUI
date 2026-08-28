//! Default application mark, Windows PE embed, and macOS `.app` packaging.
//!
//! The geometric mark is a fallback identity. Applications register their own
//! [`nana_ui_platform::WindowIcon`] at runtime, or embed a custom `.ico` from `build.rs`.
//! The macOS Dock application fits registered icons into the system icon grid
//! automatically; artwork that already carries grid margins opts out via
//! [`nana_ui_platform::WindowIcon::exact_pixels`].

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
    use image::imageops::FilterType;
    use nana_ui_platform::{WindowIcon, resolve_window_icon};

    use crate::mark::rasterize;

    /// macOS Dock 按系统格子展示图标：全幅品牌图默认收进格子（内容占画布
    /// 828/1024，居中于透明底）。已声明 `exact_pixels` 的素材原样返回。
    pub fn with_system_grid(icon: &WindowIcon) -> WindowIcon {
        if icon.uses_exact_pixels() {
            return icon.clone();
        }
        let side = icon.width.max(icon.height);
        let content = ((side as f64) * (828.0 / 1024.0)).round().max(1.0) as u32;
        let scale = content as f64 / side as f64;
        let scaled_w = ((icon.width as f64 * scale).round() as u32).max(1);
        let scaled_h = ((icon.height as f64 * scale).round() as u32).max(1);
        let source = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba.clone())
            .expect("window icon pixel count is validated");
        let resized = image::imageops::resize(&source, scaled_w, scaled_h, FilterType::Lanczos3);
        let mut canvas = image::RgbaImage::new(side, side);
        image::imageops::overlay(
            &mut canvas,
            &resized,
            ((side - scaled_w) / 2) as i64,
            ((side - scaled_h) / 2) as i64,
        );
        let mut padded = WindowIcon::from_rgba(canvas.into_raw(), side, side)
            .expect("padded canvas is a valid icon");
        padded.set_exact_pixels(true);
        padded
    }

    pub fn window_icon_from_png(bytes: &[u8]) -> Result<WindowIcon, String> {
        let image = image::load_from_memory(bytes)
            .map_err(|error| error.to_string())?
            .to_rgba8();
        let (width, height) = (image.width(), image.height());
        WindowIcon::from_rgba(image.into_raw(), width, height).map_err(|error| error.to_string())
    }

    pub fn default_window_icon() -> WindowIcon {
        WindowIcon::from_rgba(rasterize(256), 256, 256)
            .expect("default mark is valid RGBA")
            .exact_pixels(true)
    }

    pub fn resolved_application_icon(per_window: Option<&WindowIcon>) -> WindowIcon {
        resolve_window_icon(per_window).unwrap_or_else(default_window_icon)
    }
}

#[cfg(feature = "runtime")]
pub use runtime_api::{
    default_window_icon, resolved_application_icon, window_icon_from_png, with_system_grid,
};

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

    #[test]
    fn system_grid_pads_full_bleed_icons_into_transparent_margins() {
        let full_bleed = WindowIcon::from_rgba(vec![255; 64 * 64 * 4], 64, 64).unwrap();
        let padded = with_system_grid(&full_bleed);
        assert_eq!((padded.width, padded.height), (64, 64));
        assert!(padded.uses_exact_pixels());
        let pixel = |x: u32, y: u32| {
            let i = ((y * 64 + x) * 4) as usize;
            [
                padded.rgba[i],
                padded.rgba[i + 1],
                padded.rgba[i + 2],
                padded.rgba[i + 3],
            ]
        };
        assert_eq!(pixel(0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(63, 63), [0, 0, 0, 0]);
        assert_eq!(pixel(32, 32), [255, 255, 255, 255]);
    }

    #[test]
    fn exact_pixels_icons_bypass_the_system_grid() {
        let styled = WindowIcon::from_rgba(vec![7; 16 * 16 * 4], 16, 16)
            .unwrap()
            .exact_pixels(true);
        assert_eq!(with_system_grid(&styled), styled);
    }
}
