use raw_window_handle::HasWindowHandle;

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    _window: &W,
    requested: MaterialEffect,
    _appearance: Appearance,
    _fallback: FallbackColor,
) -> MaterialOutcome {
    match requested {
        MaterialEffect::Solid => MaterialOutcome::chosen_solid(),
        MaterialEffect::Transparent => MaterialOutcome::transparent(),
        MaterialEffect::Vibrancy | MaterialEffect::Mica | MaterialEffect::Acrylic => {
            MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
        }
    }
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(_window: &W) {}
