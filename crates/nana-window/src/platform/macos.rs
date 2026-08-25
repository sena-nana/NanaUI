use raw_window_handle::HasWindowHandle;
use window_vibrancy::{NSVisualEffectMaterial, apply_vibrancy, clear_vibrancy};

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    _appearance: Appearance,
    _fallback: FallbackColor,
) -> MaterialOutcome {
    match requested {
        MaterialEffect::Solid => {
            clear(window);
            MaterialOutcome::chosen_solid()
        }
        MaterialEffect::Transparent => {
            clear(window);
            MaterialOutcome::transparent()
        }
        MaterialEffect::Vibrancy => match apply_vibrancy(
            window,
            NSVisualEffectMaterial::UnderWindowBackground,
            None,
            Some(16.0),
        ) {
            Ok(()) => MaterialOutcome::native(MaterialEffect::Vibrancy),
            Err(_) => MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable),
        },
        MaterialEffect::Mica | MaterialEffect::Acrylic => {
            clear(window);
            MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
        }
    }
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(window: &W) {
    let _ = clear_vibrancy(window);
}

pub(crate) fn set_application_icon_png(png: &[u8]) {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    // AppKit requires the main thread; `mtm` is taken above.
    unsafe {
        NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
    }
}
