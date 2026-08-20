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
