//! [`DialogRecipe`]: how a dialog card sits in the window and how it is
//! divided.
//!
//! The Overlay family's recipe for the dialog card that `Dialog` and
//! `ConfirmDialog` paint over their scrim. Like every recipe it is the theme
//! answering for the component: where the card stands, how tall it may grow,
//! how round it is, how its header row spaces an icon, the title and the
//! close button, and how its header, body and footer are inset and divided
//! are design decisions, so a theme states them and a dialog only names its
//! width and fills its slots.
//!
//! The card is three sections, the way a design writes them as CSS
//! `padding` and `border`: the header row, the body and the footer's row of
//! actions, each with its own [`DialogInsets`], and an optional hairline
//! under the header and over the footer in a palette role.
//!
//! The header is one row, as a flex row with `align-items: center` lays it
//! out: `[icon] title [close]`, each in its own square or block and every
//! item centred in the row. The row is as tall as the title block, the icon
//! and [`DialogRecipe::header_min_height`]; the close button does not grow
//! it, so a close button that a busy confirmation hides does not move the
//! body under it.
//!
//! Lengths are CSS lengths resolved against the scrim, which covers the
//! window: `%` of its height, `vw` / `vh` of its size. A design that writes
//! `margin-top: 12vh; max-height: 72vh` on its card writes
//! `top: Viewport { Height, 12.0 }` and `max_height: Viewport { Height, 72.0 }`
//! here.

use super::RadiusTier;
use crate::box_layout::LengthSpec;
use crate::style_model::SemanticColorRole;

/// One section of a dialog card's insets: its CSS `padding`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogInsets {
    pub top: f32,
    pub bottom: f32,
    /// Left and right alike.
    pub inline: f32,
}

impl DialogInsets {
    pub const fn new(top: f32, bottom: f32, inline: f32) -> Self {
        Self {
            top,
            bottom,
            inline,
        }
    }

    /// CSS `padding: block inline`.
    pub const fn symmetric(block: f32, inline: f32) -> Self {
        Self::new(block, block, inline)
    }
}

/// The dialog card's placement in its scrim and how it is divided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogRecipe {
    /// Distance from the top of the scrim to the card. A card too tall to
    /// stand there moves up, never above the scrim's margin.
    pub top: LengthSpec,
    /// Tallest the card grows; content beyond it is clipped to the body.
    pub max_height: LengthSpec,
    /// The square the title's icon slot is placed in, before the title.
    pub icon_size: f32,
    /// The square the close button is placed in, after the title.
    pub close_size: f32,
    /// Space between the icon and the title, and the title and the close
    /// button: CSS `gap` on the header row.
    pub header_gap: f32,
    /// The header row's least height, the CSS `min-height` of the row. A
    /// design whose header row is as tall as its close button states that
    /// height here.
    pub header_min_height: f32,
    /// How round the card's corners are.
    pub radius: RadiusTier,
    /// Insets of the header row.
    pub header: DialogInsets,
    /// Insets of the body.
    pub body: DialogInsets,
    /// The body's bottom inset when no footer follows it. `None` keeps
    /// [`Self::body`]'s, as CSS padding would.
    pub body_bottom_alone: Option<f32>,
    /// Insets of the footer, around its row of actions.
    pub footer: DialogInsets,
    /// Space between the footer's actions.
    pub action_gap: f32,
    /// A hairline under the header, in this palette role; `None` draws none.
    pub header_divider: Option<SemanticColorRole>,
    /// A hairline over the footer, in this palette role; `None` draws none.
    pub footer_divider: Option<SemanticColorRole>,
}

impl DialogRecipe {
    /// What both built-in themes use: 90px from the top, at most 76% of the
    /// scrim's height; a 16px icon, a 28px close button, 12px apart; the
    /// card's sections inset by the spacing steps they always were, with no
    /// dividers, at the card radius.
    pub const DEFAULT: Self = Self {
        top: LengthSpec::Px(90.0),
        max_height: LengthSpec::Percent(76.0),
        icon_size: 16.0,
        close_size: 28.0,
        header_gap: 12.0,
        header_min_height: 0.0,
        radius: RadiusTier::Md,
        header: DialogInsets::new(super::space::XXL, super::space::MD, super::space::XXXL),
        body: DialogInsets::new(super::space::MD, super::space::LG, super::space::XXXL),
        body_bottom_alone: Some(super::space::XXXL),
        footer: DialogInsets::new(0.0, super::space::XXL, super::space::XXXL),
        action_gap: super::space::MD,
        header_divider: None,
        footer_divider: None,
    };

    /// The recipe's plain lengths with their token names, for validation.
    pub(super) const fn lengths(&self) -> [(&'static str, f32); 15] {
        [
            ("dialog.icon_size", self.icon_size),
            ("dialog.close_size", self.close_size),
            ("dialog.header_gap", self.header_gap),
            ("dialog.header_min_height", self.header_min_height),
            ("dialog.header.top", self.header.top),
            ("dialog.header.bottom", self.header.bottom),
            ("dialog.header.inline", self.header.inline),
            ("dialog.body.top", self.body.top),
            ("dialog.body.bottom", self.body.bottom),
            ("dialog.body.inline", self.body.inline),
            (
                "dialog.body_bottom_alone",
                match self.body_bottom_alone {
                    Some(inset) => inset,
                    None => 0.0,
                },
            ),
            ("dialog.footer.top", self.footer.top),
            ("dialog.footer.bottom", self.footer.bottom),
            ("dialog.footer.inline", self.footer.inline),
            ("dialog.action_gap", self.action_gap),
        ]
    }

    /// The body's bottom inset, with or without a footer under it.
    pub const fn body_bottom(&self, has_footer: bool) -> f32 {
        match (has_footer, self.body_bottom_alone) {
            (false, Some(inset)) => inset,
            _ => self.body.bottom,
        }
    }

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
            ..DialogRecipe::DEFAULT
        };
        assert!((recipe.top_in(1000.0, 500.0) - 60.0).abs() < 1e-3);
        assert!((recipe.max_height_in(1000.0, 500.0) - 360.0).abs() < 1e-3);
    }

    /// Without a footer the body keeps its own inset, unless the recipe
    /// names one for that case.
    #[test]
    fn the_body_bottom_follows_the_footer_like_css_padding() {
        let recipe = DialogRecipe::DEFAULT;
        assert_eq!(recipe.body_bottom(true), recipe.body.bottom);
        assert_eq!(recipe.body_bottom(false), 16.0);
        let css = DialogRecipe {
            body: DialogInsets::symmetric(12.0, 14.0),
            body_bottom_alone: None,
            ..DialogRecipe::DEFAULT
        };
        assert_eq!(css.body_bottom(false), 12.0);
        assert_eq!(css.body_bottom(true), 12.0);
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
