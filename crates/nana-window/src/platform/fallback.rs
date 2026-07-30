use raw_window_handle::HasWindowHandle;

use crate::{Appearance, FallbackColor, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    _window: &W,
    _appearance: Appearance,
    _fallback: FallbackColor,
) -> MaterialOutcome {
    MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(_window: &W) {}
