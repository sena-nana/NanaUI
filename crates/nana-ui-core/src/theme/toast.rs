//! [`ToastRecipe`]: what a toast looks like, and how a stack of toasts
//! moves.
//!
//! The Overlay family's recipe for the passive notice `Toast` paints. Like
//! every recipe it is the theme answering for the component: how round the
//! toast is, which surface and hairline it stands on, whether it is lifted,
//! how it is inset, what marks its tone before the copy and in which type
//! its title and description are set are design decisions, so a theme
//! states them and a toast only names its tone and its copy.
//!
//! The rest of the recipe is for notices: the progress row a long task shows
//! under its copy, the kind and size of its action buttons, how many lines
//! the copy wraps to, how a toast comes and goes, and how a stack closes up
//! behind one that leaves. The leaf `Toast` sets its title and its
//! description on one line each and plays no motion of its own, so it reads
//! only the look; an assembled toast (`AppContext::assemble_toast`) reads
//! the copy, progress and action rows, and a `ToastStack` the motion and the
//! stack gap.

use super::dialog::{DialogInsets, DialogTransition};
use super::recipe::RecipeText;
use super::tokens::ElevationRole;
use super::{RadiusTier, space, type_scale};
use crate::box_layout::{LengthSpec, TransformOrigin};
use crate::icon::Icon;
use crate::motion::Easing;
use crate::semantics::{ButtonKind, ControlSize, ToastTone};
use crate::style_model::SemanticColorRole;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The glyph a toast of each tone shows before its copy.
///
/// The core icon set is shell chrome, not status marks, so a theme that
/// marks tones with glyphs names its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToastGlyphs {
    pub info: Icon,
    pub success: Icon,
    pub warning: Icon,
    pub danger: Icon,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToastGlyphNames {
    info: String,
    success: String,
    warning: String,
    danger: String,
}

impl Serialize for ToastGlyphs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ToastGlyphNames {
            info: self.info.name().to_owned(),
            success: self.success.name().to_owned(),
            warning: self.warning.name().to_owned(),
            danger: self.danger.name().to_owned(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToastGlyphs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = ToastGlyphNames::deserialize(deserializer)?;
        let resolve = |slot: &str, raw: &str| -> Result<Icon, D::Error> {
            Icon::parse_name(raw).ok_or_else(|| {
                serde::de::Error::custom(format!("unknown toast glyph icon `{raw}` in `{slot}`"))
            })
        };
        Ok(Self {
            info: resolve("info", &names.info)?,
            success: resolve("success", &names.success)?,
            warning: resolve("warning", &names.warning)?,
            danger: resolve("danger", &names.danger)?,
        })
    }
}

impl ToastGlyphs {
    /// The glyph for `tone`.
    pub const fn get(self, tone: ToastTone) -> Icon {
        match tone {
            ToastTone::Info => self.info,
            ToastTone::Success => self.success,
            ToastTone::Warning => self.warning,
            ToastTone::Danger => self.danger,
        }
    }
}

/// What a toast shows before its copy to say its tone, in the tone's
/// colour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ToastIndicator {
    /// Nothing: the copy starts at the inset.
    None,
    /// A round dot `size` across.
    Dot { size: f32 },
    /// The tone's glyph in a `size` square.
    Glyph { size: f32, glyphs: ToastGlyphs },
}

impl ToastIndicator {
    /// The square the indicator takes before the copy; nothing for
    /// [`Self::None`].
    pub const fn size(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Dot { size } | Self::Glyph { size, .. } => size,
        }
    }
}

/// The progress row a toast shows under its copy while a task runs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToastProgressRecipe {
    /// The bar's thickness.
    pub height: f32,
    /// The track the fill runs along.
    pub track: SemanticColorRole,
    /// The fill.
    pub fill: SemanticColorRole,
    /// Space between the copy and the bar.
    pub top_gap: f32,
    /// Whether the row spells the percentage out beside the bar.
    pub show_percent: bool,
    /// The percentage's type.
    pub percent: RecipeText,
    /// The percentage's colour.
    pub percent_role: SemanticColorRole,
    /// The narrowest a toast showing progress gets, so its bar keeps its
    /// length while the copy beside it changes.
    pub min_width: f32,
}

impl ToastProgressRecipe {
    /// A 3px accent bar on the soft border colour, 6 under the copy, with a
    /// 12px medium percentage in the muted colour, in a toast at least 240
    /// wide.
    pub const DEFAULT: Self = Self {
        height: 3.0,
        track: SemanticColorRole::BorderSoft,
        fill: SemanticColorRole::Accent,
        top_gap: space::SM,
        show_percent: true,
        percent: RecipeText::new(type_scale::META, Some(type_scale::MEDIUM)),
        percent_role: SemanticColorRole::Muted,
        min_width: 240.0,
    };
}

impl Default for ToastProgressRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How a toast comes and goes, and how the toasts beside it move into the
/// room it makes or leaves. With reduced motion on, none of it plays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToastMotion {
    /// The entrance: a fade, and a move from [`Self::enter_offset_y`] and
    /// [`Self::enter_scale`] to where the toast rests.
    pub enter: DialogTransition,
    /// The exit: the entrance played back.
    pub exit: DialogTransition,
    /// Where the entrance starts, in px below the toast's place (CSS
    /// `translateY`); negative is above.
    pub enter_offset_y: f32,
    /// The toast's scale where the entrance starts, about [`Self::origin`].
    pub enter_scale: f32,
    /// The point a toast scales about (CSS `transform-origin` on the toast's
    /// box): its entrance, its exit and [`Self::change`]. A stack in a
    /// corner reads best growing out of that corner.
    pub origin: TransformOrigin,
    /// How a toast on screen answers its copy changing in place (a task's
    /// notice turning into its result): it springs back to rest from
    /// [`Self::change_scale`], without fading.
    pub change: DialogTransition,
    /// The toast's scale where [`Self::change`] starts; 1 plays nothing.
    pub change_scale: f32,
    /// How the toasts of a stack slide to a new place: one joining or
    /// leaving below them, or one below growing or shrinking.
    pub stack_move: DialogTransition,
}

impl ToastMotion {
    /// The 140ms ease-out fade overlays have, without a move, about the
    /// toast's centre; a change in place plays nothing, and a stack closes
    /// up over 180ms.
    pub const DEFAULT: Self = Self {
        enter: DialogTransition::new(140, Easing::EaseOutCubic),
        exit: DialogTransition::new(140, Easing::EaseOutCubic),
        enter_offset_y: 0.0,
        enter_scale: 1.0,
        origin: TransformOrigin {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        },
        change: DialogTransition::new(180, Easing::EaseOutCubic),
        change_scale: 1.0,
        stack_move: DialogTransition::new(180, Easing::EaseOutCubic),
    };

    /// Whether the toast moves at all on its way in.
    pub fn moves(&self) -> bool {
        self.enter_offset_y != 0.0 || self.enter_scale != 1.0
    }

    /// Whether a change in place plays at all.
    pub fn changes(&self) -> bool {
        self.change_scale != 1.0
    }
}

impl Default for ToastMotion {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What a toast looks like, and how a stack of toasts moves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToastRecipe {
    /// How round its corners are.
    pub radius: RadiusTier,
    /// The surface it stands on.
    pub background: SemanticColorRole,
    /// A hairline round it, as thick as the theme's hairline, in this
    /// palette role; `None` strokes none.
    pub border: Option<SemanticColorRole>,
    /// How far it is lifted off the page; `None` lays it flat.
    pub elevation: Option<ElevationRole>,
    /// Its insets: CSS `padding`.
    pub padding: DialogInsets,
    /// Space between the indicator and the copy: CSS `gap` on its row.
    pub gap: f32,
    /// What marks its tone before the copy.
    pub indicator: ToastIndicator,
    /// The title's type.
    pub title: RecipeText,
    /// Most lines a notice that wraps its title gives it; 0 does not clamp.
    /// The leaf `Toast` keeps its title on one line.
    pub title_max_lines: u16,
    /// The description's type.
    pub description: RecipeText,
    /// Most lines a notice that wraps its description gives it; 0 does not
    /// clamp. The leaf `Toast` keeps its description on one line.
    pub description_max_lines: u16,
    /// Space between the title and the description.
    pub copy_gap: f32,
    /// The progress row under the copy.
    pub progress: ToastProgressRecipe,
    /// The button kind of its actions.
    pub action_kind: ButtonKind,
    /// The size of its actions and of its dismiss button.
    pub control_size: ControlSize,
    /// The radius step of its action and dismiss buttons, independent of
    /// the toast surface. Resolves against the installed theme's metrics.
    pub control_radius: RadiusTier,
    /// How it comes, goes and moves in a stack.
    pub motion: ToastMotion,
    /// Space between the toasts of a stack.
    pub stack_gap: f32,
}

impl ToastRecipe {
    /// What both built-in themes use: a card-round surface with a hairline
    /// in the border colour and no lift, inset 10 by 12; a 6px tone dot 8
    /// before a 12px semibold title over an 11px description, 2 apart, one
    /// line each; text-kind small actions; the overlay fade; 8 between the
    /// toasts of a stack.
    pub const DEFAULT: Self = Self {
        radius: RadiusTier::Md,
        background: SemanticColorRole::Surface,
        border: Some(SemanticColorRole::Border),
        elevation: None,
        padding: DialogInsets::symmetric(space::LG, space::XL),
        gap: space::MD,
        indicator: ToastIndicator::Dot { size: space::SM },
        title: RecipeText::new(type_scale::META, Some(type_scale::SEMIBOLD)),
        title_max_lines: 1,
        description: RecipeText::new(type_scale::HINT, None),
        description_max_lines: 1,
        copy_gap: space::XXS,
        progress: ToastProgressRecipe::DEFAULT,
        action_kind: ButtonKind::Text,
        control_size: ControlSize::Small,
        control_radius: RadiusTier::Sm,
        motion: ToastMotion::DEFAULT,
        stack_gap: space::MD,
    };

    /// The recipe's plain lengths with their token names, for validation.
    pub(super) const fn lengths(&self) -> [(&'static str, f32); 10] {
        [
            ("toast.padding.top", self.padding.top),
            ("toast.padding.bottom", self.padding.bottom),
            ("toast.padding.inline", self.padding.inline),
            ("toast.gap", self.gap),
            ("toast.indicator.size", self.indicator.size()),
            ("toast.copy_gap", self.copy_gap),
            ("toast.progress.height", self.progress.height),
            ("toast.progress.top_gap", self.progress.top_gap),
            ("toast.progress.min_width", self.progress.min_width),
            ("toast.stack_gap", self.stack_gap),
        ]
    }

    /// The recipe's type with their token names, for validation.
    pub(super) const fn texts(&self) -> [(&'static str, &'static str, RecipeText); 3] {
        [
            ("toast.title.size", "toast.title.weight", self.title),
            (
                "toast.description.size",
                "toast.description.weight",
                self.description,
            ),
            (
                "toast.progress.percent.size",
                "toast.progress.percent.weight",
                self.progress.percent,
            ),
        ]
    }
}

impl Default for ToastRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in recipe is the toast as it was drawn before the theme
    /// owned it: every number is the constant it replaced.
    #[test]
    fn the_default_recipe_is_the_toast_as_it_always_looked() {
        let recipe = ToastRecipe::DEFAULT;
        assert_eq!(recipe.radius, RadiusTier::Md);
        assert_eq!(recipe.background, SemanticColorRole::Surface);
        assert_eq!(recipe.border, Some(SemanticColorRole::Border));
        assert_eq!(recipe.elevation, None);
        assert_eq!(recipe.padding, DialogInsets::new(10.0, 10.0, 12.0));
        assert_eq!(recipe.padding.top, space::LG);
        assert_eq!(recipe.padding.inline, space::XL);
        assert_eq!(recipe.gap, space::MD);
        assert_eq!(recipe.indicator, ToastIndicator::Dot { size: space::SM });
        assert_eq!(recipe.indicator.size(), 6.0);
        assert_eq!(recipe.title, RecipeText::new(12.0, Some(600)));
        assert_eq!(recipe.title_max_lines, 1);
        assert_eq!(recipe.description, RecipeText::new(11.0, None));
        assert_eq!(recipe.description_max_lines, 1);
        assert_eq!(recipe.copy_gap, space::XXS);
        assert_eq!(recipe.action_kind, ButtonKind::Text);
        assert_eq!(recipe.control_size, ControlSize::Small);
        assert_eq!(recipe.control_radius, RadiusTier::Sm);
        assert_eq!(recipe.stack_gap, space::MD);

        let progress = recipe.progress;
        assert_eq!(progress.height, 3.0);
        assert_eq!(progress.track, SemanticColorRole::BorderSoft);
        assert_eq!(progress.fill, SemanticColorRole::Accent);
        assert_eq!(progress.top_gap, space::SM);
        assert!(progress.show_percent);
        assert_eq!(progress.percent, RecipeText::new(12.0, Some(500)));
        assert_eq!(progress.percent_role, SemanticColorRole::Muted);
        assert_eq!(progress.min_width, 240.0);

        let motion = recipe.motion;
        assert_eq!(
            motion.enter,
            DialogTransition::new(140, Easing::EaseOutCubic)
        );
        assert_eq!(motion.exit, motion.enter);
        assert_eq!(
            motion.stack_move,
            DialogTransition::new(180, Easing::EaseOutCubic)
        );
        assert!(!motion.moves());
        assert_eq!(
            motion.enter.duration(),
            crate::motion::OVERLAY_FADE,
            "a toast fades like the overlays it floats with"
        );
    }

    /// No mark takes no room; a dot and a glyph take their square, and the
    /// glyph follows the tone.
    #[test]
    fn the_indicator_takes_its_square_and_a_glyph_follows_the_tone() {
        assert_eq!(ToastIndicator::None.size(), 0.0);
        let glyphs = ToastGlyphs {
            info: Icon::About,
            success: Icon::ShieldCheck,
            warning: Icon::Eye,
            danger: Icon::Close,
        };
        let glyph = ToastIndicator::Glyph { size: 16.0, glyphs };
        assert_eq!(glyph.size(), 16.0);
        assert_eq!(glyphs.get(ToastTone::Info), Icon::About);
        assert_eq!(glyphs.get(ToastTone::Success), Icon::ShieldCheck);
        assert_eq!(glyphs.get(ToastTone::Warning), Icon::Eye);
        assert_eq!(glyphs.get(ToastTone::Danger), Icon::Close);
    }
}
