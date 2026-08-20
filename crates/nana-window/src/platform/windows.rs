use raw_window_handle::HasWindowHandle;
use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    clear(window);
    match requested {
        MaterialEffect::Solid => MaterialOutcome::chosen_solid(),
        MaterialEffect::Transparent => MaterialOutcome::transparent(),
        MaterialEffect::Mica => {
            let dark = matches!(appearance, Appearance::Dark);
            if apply_mica(window, Some(dark)).is_ok() {
                MaterialOutcome::native(MaterialEffect::Mica)
            } else {
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
        }
        MaterialEffect::Acrylic => {
            if apply_acrylic(window, Some(fallback.tuple())).is_ok() {
                MaterialOutcome::native(MaterialEffect::Acrylic)
            } else {
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
        }
        MaterialEffect::Vibrancy => {
            MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
        }
    }
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(window: &W) {
    let _ = clear_mica(window);
    let _ = clear_acrylic(window);
}
