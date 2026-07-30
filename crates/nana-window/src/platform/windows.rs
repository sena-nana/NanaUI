use raw_window_handle::HasWindowHandle;
use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    let dark = matches!(appearance, Appearance::Dark);
    if apply_mica(window, Some(dark)).is_ok() {
        return MaterialOutcome::native(MaterialEffect::Mica);
    }
    if apply_acrylic(window, Some(fallback.tuple())).is_ok() {
        return MaterialOutcome::native(MaterialEffect::Acrylic);
    }
    MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(window: &W) {
    let _ = clear_mica(window);
    let _ = clear_acrylic(window);
}
