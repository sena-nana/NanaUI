//! Platforms without a shadow backend: the outcome says so.

use raw_window_handle::HasWindowHandle;

use super::{ShadowApplied, ShadowShape, ShadowStyle, ShadowWork};

/// Server-side decorations (or none) decide; nothing here is observable.
pub(super) fn set_native<W: HasWindowHandle + ?Sized>(
    _window: &W,
    _enabled: bool,
) -> ShadowApplied {
    ShadowApplied::Unsupported
}

pub(super) struct Companion;

impl Companion {
    pub(super) fn create<W: HasWindowHandle + ?Sized>(
        _window: &W,
        _style: ShadowStyle,
        _shape: ShadowShape,
        _work: &mut ShadowWork,
    ) -> Option<Result<Self, ()>> {
        None
    }

    pub(super) fn update<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        _style: ShadowStyle,
        _shape: ShadowShape,
        _visible: bool,
        _work: &mut ShadowWork,
    ) -> bool {
        false
    }

    pub(super) fn rescale<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        _work: &mut ShadowWork,
    ) {
    }

    pub(super) fn add_work(&self, _work: &mut ShadowWork) {}
}
