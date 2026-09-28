//! Desktop window shadows.
//!
//! A window's shadow is either the platform's own frame shadow, or — for a
//! transparent window whose visible body is a card inside it — a *companion*:
//! a platform-private, click-through window placed directly behind the
//! primary that draws only the shadow of that card. The primary window's
//! frame, client geometry and native handle never change for it, and the
//! companion is never a NanaUI window (no id, events, focus or input).
//!
//! The companion holds retained compositor content (a `CALayer` shadow path
//! on macOS, a DirectComposition nine-slice on Windows). Moving the primary
//! moves it; resizing updates its geometry; only a style or scale change
//! rasterizes anything. [`ShadowWork`] counts each kind, so a gate can prove
//! that a static window does no shadow work at all.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use fallback as platform;

use raw_window_handle::HasWindowHandle;

/// Shadow parameters in logical pixels; `color` is straight sRGB RGBA.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    pub color: [f32; 4],
    pub offset: [f32; 2],
    pub blur: f32,
    pub spread: f32,
}

impl ShadowStyle {
    /// How far the shadow can reach past the shape on any side, in logical
    /// pixels: what the companion has to extend beyond it.
    pub fn margin(self) -> f64 {
        let reach = f64::from(self.blur.max(0.0)) + f64::from(self.spread.max(0.0));
        reach + f64::from(self.offset[0].abs()).max(f64::from(self.offset[1].abs())) + 1.0
    }
}

/// The body the shadow follows: a rounded rectangle in logical client
/// coordinates, `[x, y, width, height]`, radii from the top-left clockwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowShape {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
}

impl ShadowShape {
    /// The one corner radius a platform path that takes a single radius uses.
    pub fn radius(self) -> f64 {
        let largest = self.radii.iter().fold(0.0_f32, |a, b| a.max(*b));
        f64::from(
            largest
                .min(self.rect[2] / 2.0)
                .min(self.rect[3] / 2.0)
                .max(0.0),
        )
    }
}

/// What the host decided this window's shadow is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShadowPlan {
    /// No shadow at all.
    Disable,
    /// The platform's own window shadow.
    Native,
    /// A companion drawing `style` around `shape`.
    Companion {
        style: ShadowStyle,
        shape: ShadowShape,
    },
}

/// What was actually applied, as observed on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowApplied {
    Disabled,
    Native,
    Companion,
    /// `Disable` was asked, but the platform's frame shadow stays (Windows DWM
    /// on a framed window).
    NativeNotRemovable,
    /// The platform cannot do what was asked here; nothing is drawn.
    Unsupported,
    /// The platform failed; nothing is drawn.
    Failed,
}

/// Work counters. See the module docs for which change may raise which.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShadowWork {
    pub shape_updates: u64,
    pub effect_updates: u64,
    pub rasterizations: u64,
    pub resource_recreates: u64,
    pub companion_moves: u64,
    pub companion_resizes: u64,
}

/// One window's shadow: the plan in force, its companion if any, and the
/// work done for it. Dropping it removes the companion.
#[derive(Default)]
pub struct WindowShadowState {
    companion: Option<platform::Companion>,
    applied: Option<ShadowApplied>,
    plan: Option<ShadowPlan>,
    work: ShadowWork,
}

impl std::fmt::Debug for WindowShadowState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowShadowState")
            .field("applied", &self.applied)
            .field("companion", &self.companion.is_some())
            .field("work", &self.work)
            .finish()
    }
}

impl WindowShadowState {
    /// Apply `plan` to `window`. Re-applying the plan in force does nothing;
    /// a companion whose style or shape changed is updated in place.
    pub fn apply<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
        plan: ShadowPlan,
    ) -> ShadowApplied {
        if self.plan == Some(plan)
            && let Some(applied) = self.applied
        {
            return applied;
        }
        let applied = match plan {
            ShadowPlan::Disable => {
                self.companion = None;
                platform::set_native(window, false)
            }
            ShadowPlan::Native => {
                self.companion = None;
                platform::set_native(window, true)
            }
            ShadowPlan::Companion { style, shape } => {
                // The companion replaces the platform's own shadow.
                let _ = platform::set_native(window, false);
                self.update_companion(window, style, shape, true)
            }
        };
        self.plan = Some(plan);
        self.applied = Some(applied);
        applied
    }

    /// The visible body moved or resized, or the window was hidden or shown.
    /// Only a companion has anything to do.
    pub fn set_shape<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
        shape: ShadowShape,
        visible: bool,
    ) {
        let Some(ShadowPlan::Companion { style, shape: old }) = self.plan else {
            return;
        };
        if old != shape {
            self.work.shape_updates += 1;
        }
        let applied = self.update_companion(window, style, shape, visible);
        self.plan = Some(ShadowPlan::Companion { style, shape });
        self.applied = Some(applied);
    }

    /// The window's backing scale changed: companion content is rasterized
    /// or pathed at device resolution.
    pub fn rescale<W: HasWindowHandle + ?Sized>(&mut self, window: &W) {
        if let Some(companion) = self.companion.as_mut() {
            companion.rescale(window, &mut self.work);
        }
    }

    fn update_companion<W: HasWindowHandle + ?Sized>(
        &mut self,
        window: &W,
        style: ShadowStyle,
        shape: ShadowShape,
        visible: bool,
    ) -> ShadowApplied {
        if let Some(companion) = self.companion.as_mut() {
            return if companion.update(window, style, shape, visible, &mut self.work) {
                ShadowApplied::Companion
            } else {
                self.companion = None;
                ShadowApplied::Failed
            };
        }
        match platform::Companion::create(window, style, shape, &mut self.work) {
            Some(Ok(companion)) => {
                self.work.resource_recreates += 1;
                self.companion = Some(companion);
                if let Some(companion) = self.companion.as_mut() {
                    companion.update(window, style, shape, visible, &mut self.work);
                }
                ShadowApplied::Companion
            }
            Some(Err(())) => ShadowApplied::Failed,
            None => ShadowApplied::Unsupported,
        }
    }

    pub fn applied(&self) -> Option<ShadowApplied> {
        self.applied
    }

    pub fn has_companion(&self) -> bool {
        self.companion.is_some()
    }

    pub fn work(&self) -> ShadowWork {
        let mut work = self.work;
        if let Some(companion) = self.companion.as_ref() {
            companion.add_work(&mut work);
        }
        work
    }

    /// Remove the companion now (the primary is closing).
    pub fn release(&mut self) {
        self.companion = None;
        self.plan = None;
        self.applied = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_covers_blur_spread_and_the_larger_offset() {
        let style = ShadowStyle {
            color: [0.0, 0.0, 0.0, 0.25],
            offset: [2.0, -6.0],
            blur: 16.0,
            spread: 3.0,
        };
        assert_eq!(style.margin(), 16.0 + 3.0 + 6.0 + 1.0);
    }

    #[test]
    fn a_single_radius_never_exceeds_half_the_shape() {
        let shape = ShadowShape {
            rect: [0.0, 0.0, 40.0, 10.0],
            radii: [4.0, 12.0, 0.0, 0.0],
        };
        assert_eq!(shape.radius(), 5.0);
    }
}
