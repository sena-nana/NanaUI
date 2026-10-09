//! [`DialogRecipe`]: how a dialog card sits in the window.
//!
//! The Overlay family's recipe for the dialog card that `Dialog` and
//! `ConfirmDialog` paint over their scrim. Like every recipe it is the theme
//! answering for the component: where the card stands and how tall it may
//! grow are design decisions, so a theme states them and a dialog only names
//! its width.
//!
//! Lengths are CSS lengths resolved against the scrim, which covers the
//! window: `%` of its height, `vw` / `vh` of its size. A design that writes
//! `margin-top: 12vh; max-height: 72vh` on its card writes
//! `top: Viewport { Height, 12.0 }` and `max_height: Viewport { Height, 72.0 }`
//! here.

use crate::box_layout::LengthSpec;

/// The dialog card's placement in its scrim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogRecipe {
    /// Distance from the top of the scrim to the card. A card too tall to
    /// stand there moves up, never above the scrim's margin.
    pub top: LengthSpec,
    /// Tallest the card grows; content beyond it is clipped to the body.
    pub max_height: LengthSpec,
}

impl DialogRecipe {
    /// What both built-in themes use: 90px from the top, at most 76% of the
    /// scrim's height.
    pub const DEFAULT: Self = Self {
        top: LengthSpec::Px(90.0),
        max_height: LengthSpec::Percent(76.0),
    };

    /// [`Self::top`] over a scrim of `width` × `height`.
    pub fn top_in(&self, width: f32, height: f32) -> f32 {
        resolve(self.top, width, height).unwrap_or(0.0)
    }

    /// [`Self::max_height`] over a scrim of `width` × `height`.
    pub fn max_height_in(&self, width: f32, height: f32) -> f32 {
        resolve(self.max_height, width, height).unwrap_or(height)
    }
}

impl Default for DialogRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A length over a scrim of `width` × `height`: `%` of its height, viewport
/// units of its size. `None` when the length names no size.
fn resolve(length: LengthSpec, width: f32, height: f32) -> Option<f32> {
    length
        .resolve_with(Some(height), Some((width, height)))
        .filter(|value| value.is_finite())
        .map(|value| value.max(0.0))
}

/// Whether `length` is one a theme may give a dialog: it names a size, and
/// that size is finite and not negative over a plain window.
pub(super) fn is_dialog_length(length: LengthSpec) -> bool {
    length
        .resolve_with(Some(1000.0), Some((1000.0, 1000.0)))
        .is_some_and(|value| value.is_finite() && value >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_layout::ViewportAxis;

    #[test]
    fn the_default_recipe_is_the_placement_dialogs_always_had() {
        let recipe = DialogRecipe::DEFAULT;
        assert_eq!(recipe.top_in(800.0, 600.0), 90.0);
        assert!((recipe.max_height_in(800.0, 600.0) - 456.0).abs() < 1e-3);
    }

    #[test]
    fn viewport_lengths_read_the_scrim() {
        let recipe = DialogRecipe {
            top: LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 12.0,
            },
            max_height: LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 72.0,
            },
        };
        assert!((recipe.top_in(1000.0, 500.0) - 60.0).abs() < 1e-3);
        assert!((recipe.max_height_in(1000.0, 500.0) - 360.0).abs() < 1e-3);
    }

    #[test]
    fn only_a_finite_non_negative_size_is_a_dialog_length() {
        assert!(is_dialog_length(LengthSpec::Px(0.0)));
        assert!(is_dialog_length(LengthSpec::Percent(76.0)));
        assert!(!is_dialog_length(LengthSpec::Px(-1.0)));
        assert!(!is_dialog_length(LengthSpec::Px(f32::NAN)));
        assert!(!is_dialog_length(LengthSpec::Auto));
    }
}
