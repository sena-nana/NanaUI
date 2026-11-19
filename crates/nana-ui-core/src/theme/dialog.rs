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
//!
//! The title is the recipe's too: its type, whether a danger dialog speaks
//! it in the danger tone, and how far it may run. A design whose header
//! stands on a narrower shape than the card — a tab rising over the body —
//! keeps the title inside that shape with [`DialogRecipe::title_max_width`]
//! and starts its rule where the heading does with
//! [`DialogDividerInset::Content`].

use super::RadiusTier;
use super::recipe::RecipeText;
use crate::box_layout::LengthSpec;
use crate::motion::Easing;
use crate::style_model::SemanticColorRole;

use serde::{Deserialize, Serialize};

/// One transition of a dialog's entrance and exit: how long, along which
/// curve. CSS `transition: <property> <duration> <timing-function>`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DialogTransition {
    pub millis: u16,
    pub easing: Easing,
}

impl DialogTransition {
    pub const fn new(millis: u16, easing: Easing) -> Self {
        Self { millis, easing }
    }

    pub const fn duration(self) -> std::time::Duration {
        std::time::Duration::from_millis(self.millis as u64)
    }
}

/// How a dialog comes and goes: the scrim fades, and the card fades and
/// moves into place from a little above, a little smaller. The exit plays
/// the same transitions back. With reduced motion on, none of it plays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DialogMotion {
    /// The scrim's fade.
    pub scrim: DialogTransition,
    /// The card's fade, with its contents.
    pub card_fade: DialogTransition,
    /// The card's move, from [`Self::enter_offset_y`] and
    /// [`Self::enter_scale`] to where it rests.
    pub card_move: DialogTransition,
    /// Where the card's entrance starts, in px below its place (CSS
    /// `translateY`); negative is above.
    pub enter_offset_y: f32,
    /// The card's scale where its entrance starts, about its own centre.
    pub enter_scale: f32,
}

impl DialogMotion {
    /// The fade every overlay has always had: scrim and card together,
    /// 140ms, easing out; the card does not move.
    pub const DEFAULT: Self = Self {
        scrim: DialogTransition::new(140, Easing::EaseOutCubic),
        card_fade: DialogTransition::new(140, Easing::EaseOutCubic),
        card_move: DialogTransition::new(140, Easing::EaseOutCubic),
        enter_offset_y: 0.0,
        enter_scale: 1.0,
    };

    /// Whether the card moves at all on its way in.
    pub fn moves(&self) -> bool {
        self.enter_offset_y != 0.0 || self.enter_scale != 1.0
    }
}

impl Default for DialogMotion {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One section of a dialog card's insets: its CSS `padding`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// Where a dialog card's dividers start and end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DialogDividerInset {
    /// Edge to edge of the card.
    #[default]
    Edge,
    /// Inside the section's inline inset — the header's under the header,
    /// the footer's over the footer — the way a rule under a heading starts
    /// where the heading's text does.
    Content,
}

/// Where a dialog card stands in its scrim, up and down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DialogAlign {
    /// [`DialogRecipe::top`] from the scrim's top: the card grows downward
    /// as its content does.
    #[default]
    Top,
    /// Midway down the scrim, as far from its top as from its bottom (a
    /// flex column with `justify-content: center`): the card grows both
    /// ways as its content does.
    Center,
}

/// The dialog card's placement in its scrim and how it is divided.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DialogRecipe {
    /// Distance from the top of the scrim to the card, when [`Self::align`]
    /// is [`DialogAlign::Top`]. A card too tall to stand there moves up,
    /// never above the scrim's margin.
    pub top: LengthSpec,
    /// Where the card stands, up and down: [`Self::top`] from the scrim's
    /// top, or centred in it. Either way it stays inside the scrim's margin.
    pub align: DialogAlign,
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
    /// Where the two hairlines start and end.
    pub divider_inset: DialogDividerInset,
    /// The title's type. A drawer keeps its own.
    pub title: RecipeText,
    /// Whether a danger dialog speaks its title in the status recipe's
    /// danger tone. A design that marks danger with the title's icon and the
    /// confirm button keeps the title in the overlay's text colour.
    pub danger_title: bool,
    /// Widest the header's text — the title and its description — may run,
    /// as a CSS length: `%` of the card's width, `vw` / `vh` of the scrim.
    /// `None` lets it run to the close button.
    pub title_max_width: Option<LengthSpec>,
    /// How the dialog comes and goes.
    pub motion: DialogMotion,
}

impl DialogRecipe {
    /// What both built-in themes use: 90px from the top, at most 76% of the
    /// scrim's height; a 16px icon, a 28px close button, 12px apart; the
    /// card's sections inset by the spacing steps they always were, with no
    /// dividers, at the card radius; a 14px semibold title that turns to the
    /// danger tone on a danger dialog and runs to the close button.
    pub const DEFAULT: Self = Self {
        top: LengthSpec::Px(90.0),
        align: DialogAlign::Top,
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
        divider_inset: DialogDividerInset::Edge,
        title: RecipeText::new(
            super::type_scale::SECTION,
            Some(super::type_scale::SEMIBOLD),
        ),
        danger_title: true,
        title_max_width: None,
        motion: DialogMotion::DEFAULT,
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

    /// [`Self::title_max_width`] for a card `card_width` wide over a scrim
    /// of `width` × `height`: `%` reads the card, viewport units the scrim.
    pub fn title_max_width_in(&self, card_width: f32, width: f32, height: f32) -> Option<f32> {
        self.title_max_width?
            .resolve_with(Some(card_width), Some((width, height)))
            .filter(|value| value.is_finite())
            .map(|value| value.max(0.0))
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
        assert_eq!(recipe.align, DialogAlign::Top);
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

    /// The title is the one built-in dialogs always drew, and nothing
    /// narrows it until a theme says so.
    #[test]
    fn the_default_title_is_the_semibold_section_title_dialogs_always_had() {
        let recipe = DialogRecipe::DEFAULT;
        assert_eq!(
            recipe.title,
            RecipeText::new(super::super::type_scale::SECTION, Some(600))
        );
        assert!(recipe.danger_title);
        assert_eq!(recipe.divider_inset, DialogDividerInset::Edge);
        assert_eq!(recipe.title_max_width_in(420.0, 1280.0, 800.0), None);
    }

    /// `%` reads the card, so a tab drawn at a share of the card's width
    /// and the title kept inside it scale together; viewport units read
    /// the scrim like every other dialog length.
    #[test]
    fn the_title_width_reads_the_card_for_percent_and_the_scrim_for_viewport_units() {
        let tabbed = DialogRecipe {
            title_max_width: Some(LengthSpec::Percent(50.0)),
            ..DialogRecipe::DEFAULT
        };
        assert_eq!(tabbed.title_max_width_in(420.0, 1280.0, 800.0), Some(210.0));
        let viewport = DialogRecipe {
            title_max_width: Some(LengthSpec::Viewport {
                axis: ViewportAxis::Width,
                value: 10.0,
            }),
            ..DialogRecipe::DEFAULT
        };
        assert_eq!(
            viewport.title_max_width_in(420.0, 1280.0, 800.0),
            Some(128.0)
        );
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
