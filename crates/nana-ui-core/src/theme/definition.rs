//! [`ThemeDefinition`]: the authoring form of a NanaUI design system.
//!
//! A definition is what a theme author writes and what tooling diffs. It is
//! not what the runtime reads — [`compile`](ThemeDefinition::compile) turns it
//! into a validated [`CompiledTheme`](super::CompiledTheme), and the runtime
//! reads that. The split exists for two reasons that the Phase 0 audit made
//! concrete:
//!
//! 1. **Fail-closed.** A definition may leave a recipe slot unset. A compiled
//!    theme may not. Rejecting a half-authored theme at install is cheaper
//!    than discovering it as a missing colour three screens in.
//! 2. **No string lookups on the runtime path.** Names exist here, for
//!    identity and diagnostics. What crosses into the runtime is enum-indexed
//!    structs.
//!
//! ## What this is *not*
//!
//! It is not a second copy of [`SemanticPalette`] or [`ThemeMetrics`]. Both
//! are held directly, by value, as the colour and control-metric token
//! categories. Issue #102 forbids a `ThemeColors2`, and the way to not write
//! one is to hold the original.

use super::recipe::{
    BUTTON_KINDS, ButtonRecipe, ButtonRecipeDraft, ButtonVariantDraft, ButtonVariantRecipe,
    CompiledRecipes, ComponentRecipe, ComponentRecipeDraft, ComponentRecipeId,
    ComponentThemeRegistry, StatusRecipe, button_index, button_kind_name,
};
use super::tokens::{
    AccentRamp, BorderTokens, EffectTokens, MotionTokens, OpacityTokens, SpacingTokens,
    SurfaceTokens, TypographyTokens,
};
use super::{ThemeMetrics, ThemeMode, UI_METRICS};
use crate::style_model::{SemanticColor, SemanticColorRole, SemanticPalette, StyleModelRef};

/// Stable identity of a theme.
///
/// A `&'static str` rather than an interned handle or a hash: a theme id is
/// read by humans in diagnostics far more often than it is compared, and this
/// phase does not load themes from files (that is a later phase's theme
/// package, which is also where a non-static id starts to make sense).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThemeId(&'static str);

impl ThemeId {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ThemeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Which version of the theme *contract* a definition was written against.
///
/// Major changes when a token category is removed or its meaning changes, so a
/// definition written for an older major is rejected rather than silently
/// reinterpreted. Minor changes when a category is added with a default, which
/// an older definition can still compile against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThemeSchemaVersion {
    pub major: u16,
    pub minor: u16,
}

impl ThemeSchemaVersion {
    /// The contract this build of NanaUI compiles.
    pub const CURRENT: Self = Self::new(1, 0);

    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Whether [`Self::CURRENT`] can compile a definition written for `self`.
    pub const fn is_supported(self) -> bool {
        self.compiles_under(Self::CURRENT)
    }

    /// Whether a build that speaks `contract` can compile a definition
    /// written for `self`: same major, and not a newer minor.
    ///
    /// Split out rather than inlined so the minor check is a comparison of two
    /// values. Inline against [`Self::CURRENT`] it reads as `minor <= 0`,
    /// which is both degenerate today and the thing that has to keep working
    /// on the day `CURRENT.minor` becomes 1.
    pub const fn compiles_under(self, contract: Self) -> bool {
        self.major == contract.major && self.minor <= contract.minor
    }
}

impl std::fmt::Display for ThemeSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Monotonic revision of one theme's token values.
///
/// Identity answers *which* theme; generation answers *which revision of it*.
/// Together they are the O(1) question "is this the same theme I already
/// resolved against" — which is what a cached compile, and later Issue #100
/// §7's dependency-scoped invalidation, need to ask without walking tokens.
///
/// The policy is explicit so it can be relied on: **bump on any change to a
/// token value, a recipe slot, or the mode.** Not on a comment, not on a
/// rename. A theme whose values changed without a bump is a bug in the theme,
/// which is why [`ThemeDefinition`] exposes [`bump`](ThemeDefinition::bump)
/// and every mutator in this module calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ThemeGeneration(pub u32);

impl ThemeGeneration {
    pub const FIRST: Self = Self(1);

    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl std::fmt::Display for ThemeGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who a compiled theme is. Copy, so a runtime handle can carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThemeIdentity {
    pub id: ThemeId,
    pub schema: ThemeSchemaVersion,
    pub generation: ThemeGeneration,
}

impl std::fmt::Display for ThemeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}/schema {}", self.id, self.generation, self.schema)
    }
}

/// The theme author's own palette, before it becomes semantic roles.
///
/// Issue #100 §2: an author may organise colour however they like, but no
/// NanaUI component may depend on that organisation. Nothing outside
/// [`ThemeDefinition::compile`] reads this, which is the enforcement — a
/// component cannot name `blue500` because there is no path from a component
/// to this struct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoundationTokens {
    pub accent: AccentRamp,
}

impl FoundationTokens {
    pub const fn for_mode(mode: ThemeMode) -> Self {
        Self {
            accent: AccentRamp::for_mode(mode),
        }
    }
}

/// The token categories, bundled.
///
/// Named `DesignTokens` and not `ThemeTokens` on purpose: `nana_ui::theme::
/// ThemeTokens` is the host-side install bundle and keeps that name. One name,
/// one type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignTokens {
    /// Author-private base values. See [`FoundationTokens`].
    pub foundation: FoundationTokens,
    /// Semantic colour roles. The stable contract components depend on.
    pub palette: SemanticPalette,
    /// Radius, control metrics, panel / field insets, scrollbar geometry.
    pub metrics: ThemeMetrics,
    pub spacing: SpacingTokens,
    pub border: BorderTokens,
    pub opacity: OpacityTokens,
    /// Chrome strip colour. `None` follows [`SemanticPalette::surface`], which
    /// is what lets a translucent sidebar keep an opaque title bar.
    pub titlebar: Option<SemanticColor>,
}

impl DesignTokens {
    pub const fn for_mode(mode: ThemeMode) -> Self {
        Self {
            foundation: FoundationTokens::for_mode(mode),
            palette: SemanticPalette::for_mode(mode),
            metrics: UI_METRICS,
            spacing: SpacingTokens::DEFAULT,
            border: BorderTokens::DEFAULT,
            opacity: OpacityTokens::for_mode(mode),
            titlebar: None,
        }
    }

    /// The chrome colour this bundle resolves to.
    pub const fn titlebar_color(&self) -> SemanticColor {
        match self.titlebar {
            Some(color) => color,
            None => self.palette.surface,
        }
    }
}

/// A NanaUI design system, in authoring form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeDefinition {
    pub id: ThemeId,
    pub schema: ThemeSchemaVersion,
    pub generation: ThemeGeneration,
    pub mode: ThemeMode,
    pub tokens: DesignTokens,
    pub typography: TypographyTokens,
    pub motion: MotionTokens,
    pub effects: EffectTokens,
    pub surfaces: SurfaceTokens,
    pub components: ComponentThemeRegistry,
}

/// Why a definition could not become a runtime theme.
///
/// Every variant names the theme, because the first question on seeing one of
/// these is "which theme did that".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeCompileError {
    /// The definition was written against a contract this build cannot honour.
    UnsupportedSchema {
        theme: ThemeId,
        found: ThemeSchemaVersion,
        supported: ThemeSchemaVersion,
    },
    /// A theme that says it never changed, yet has no revision at all.
    ZeroGeneration { theme: ThemeId },
    /// NaN or infinity reached a token.
    NotFinite { theme: ThemeId, token: &'static str },
    /// A length token went negative. A negative radius is not a design choice.
    NegativeLength {
        theme: ThemeId,
        token: &'static str,
        value: i32,
    },
    /// An alpha left `0.0..=1.0`.
    AlphaOutOfRange {
        theme: ThemeId,
        token: &'static str,
        value: i32,
    },
    /// A font size left the range a shaper can do anything sensible with.
    FontSizeOutOfRange {
        theme: ThemeId,
        token: &'static str,
        value: i32,
    },
    /// A font weight left the CSS `1..=1000` range.
    FontWeightOutOfRange {
        theme: ThemeId,
        token: &'static str,
        value: u16,
    },
    /// A transition that never advances is a disabled transition written by
    /// accident; say so rather than freezing a control mid-fade.
    ZeroDuration { theme: ThemeId, token: &'static str },
    /// A component family has no recipe.
    MissingRecipe {
        theme: ThemeId,
        component: ComponentRecipeId,
        slot: &'static str,
    },
    /// A button variant has no recipe slot.
    MissingButtonSlot {
        theme: ThemeId,
        variant: &'static str,
        slot: &'static str,
    },
    /// The registry has no status-tone mapping.
    MissingStatusRecipe { theme: ThemeId },
}

impl std::fmt::Display for ThemeCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema {
                theme,
                found,
                supported,
            } => write!(
                f,
                "theme `{theme}` targets schema {found}, this build compiles {supported}"
            ),
            Self::ZeroGeneration { theme } => {
                write!(
                    f,
                    "theme `{theme}` has generation 0; the first revision is 1"
                )
            }
            Self::NotFinite { theme, token } => {
                write!(f, "theme `{theme}` token `{token}` is not finite")
            }
            Self::NegativeLength {
                theme,
                token,
                value,
            } => write!(
                f,
                "theme `{theme}` token `{token}` is negative ({value} thousandths)"
            ),
            Self::AlphaOutOfRange {
                theme,
                token,
                value,
            } => write!(
                f,
                "theme `{theme}` alpha `{token}` is outside 0..=1 ({value} thousandths)"
            ),
            Self::FontSizeOutOfRange {
                theme,
                token,
                value,
            } => write!(
                f,
                "theme `{theme}` font size `{token}` is outside 1..=512 ({value} thousandths)"
            ),
            Self::FontWeightOutOfRange {
                theme,
                token,
                value,
            } => write!(
                f,
                "theme `{theme}` font weight `{token}` is outside 1..=1000 ({value})"
            ),
            Self::ZeroDuration { theme, token } => {
                write!(f, "theme `{theme}` duration `{token}` is zero")
            }
            Self::MissingRecipe {
                theme,
                component,
                slot,
            } => write!(
                f,
                "theme `{theme}` leaves `{}.{slot}` unset",
                component.name()
            ),
            Self::MissingButtonSlot {
                theme,
                variant,
                slot,
            } => write!(f, "theme `{theme}` leaves `Button.{variant}.{slot}` unset"),
            Self::MissingStatusRecipe { theme } => {
                write!(f, "theme `{theme}` has no status-tone recipe")
            }
        }
    }
}

impl std::error::Error for ThemeCompileError {}

/// Milli-units, so an error can print a float without formatting one.
fn thousandths(value: f32) -> i32 {
    (value * 1000.0) as i32
}

struct Check<'a> {
    theme: ThemeId,
    error: &'a mut Option<ThemeCompileError>,
}

impl Check<'_> {
    fn fail(&mut self, error: ThemeCompileError) {
        if self.error.is_none() {
            *self.error = Some(error);
        }
    }

    fn finite(&mut self, token: &'static str, value: f32) -> bool {
        if value.is_finite() {
            return true;
        }
        self.fail(ThemeCompileError::NotFinite {
            theme: self.theme,
            token,
        });
        false
    }

    fn length(&mut self, token: &'static str, value: f32) {
        if self.finite(token, value) && value < 0.0 {
            self.fail(ThemeCompileError::NegativeLength {
                theme: self.theme,
                token,
                value: thousandths(value),
            });
        }
    }

    fn alpha(&mut self, token: &'static str, value: f32) {
        if self.finite(token, value) && !(0.0..=1.0).contains(&value) {
            self.fail(ThemeCompileError::AlphaOutOfRange {
                theme: self.theme,
                token,
                value: thousandths(value),
            });
        }
    }

    fn color(&mut self, token: &'static str, color: SemanticColor) {
        self.alpha(token, color.a);
        for channel in [color.r, color.g, color.b] {
            if self.finite(token, channel) && !(0.0..=1.0).contains(&channel) {
                self.fail(ThemeCompileError::AlphaOutOfRange {
                    theme: self.theme,
                    token,
                    value: thousandths(channel),
                });
            }
        }
    }

    fn font_size(&mut self, token: &'static str, value: f32) {
        if self.finite(token, value) && !(1.0..=512.0).contains(&value) {
            self.fail(ThemeCompileError::FontSizeOutOfRange {
                theme: self.theme,
                token,
                value: thousandths(value),
            });
        }
    }

    fn font_weight(&mut self, token: &'static str, value: u16) {
        if !(1..=1000).contains(&value) {
            self.fail(ThemeCompileError::FontWeightOutOfRange {
                theme: self.theme,
                token,
                value,
            });
        }
    }

    fn duration(&mut self, token: &'static str, millis: u16) {
        if millis == 0 {
            self.fail(ThemeCompileError::ZeroDuration {
                theme: self.theme,
                token,
            });
        }
    }
}

impl ThemeDefinition {
    /// Bump the revision. Call after changing any token value.
    pub const fn bump(mut self) -> Self {
        self.generation = self.generation.next();
        self
    }

    /// Replace the semantic palette.
    ///
    /// The foundation accent ramp is re-read from the new palette rather than
    /// left behind: a definition whose foundation claims one accent while its
    /// semantic layer paints another is a definition that lies to the next
    /// person who reads it. Nothing about the palette itself is changed — see
    /// [`AccentRamp::from_palette`].
    pub const fn with_palette(mut self, palette: SemanticPalette) -> Self {
        self.tokens.foundation.accent = AccentRamp::from_palette(&palette);
        self.tokens.palette = palette;
        self.bump()
    }

    /// Replace the control metrics (radius, heights, insets, scrollbar).
    pub const fn with_metrics(mut self, metrics: ThemeMetrics) -> Self {
        self.tokens.metrics = metrics;
        self.bump()
    }

    /// Pin the chrome strip colour. `None` follows the surface role.
    pub const fn with_titlebar(mut self, titlebar: Option<SemanticColor>) -> Self {
        self.tokens.titlebar = titlebar;
        self.bump()
    }

    /// Re-derive the whole accent family from one ramp.
    ///
    /// This is the foundation layer earning its place: setting
    /// `palette.accent` alone leaves `accent_soft` and friends on the previous
    /// hue, because those three are the base accent at three alphas. Going
    /// through the ramp keeps the family consistent.
    pub const fn with_accent(mut self, accent: AccentRamp) -> Self {
        self.tokens.foundation.accent = accent;
        self.tokens.palette = self.tokens.palette.with_accent_ramp(accent);
        self.bump()
    }

    /// Give the definition its own identity. Use when deriving a theme from a
    /// built-in: a variant of NanaDark is not NanaDark.
    pub const fn with_id(mut self, id: ThemeId) -> Self {
        self.id = id;
        self.generation = ThemeGeneration::FIRST;
        self
    }

    /// Validate and lower into the form the runtime reads.
    pub fn compile(&self) -> Result<super::CompiledTheme, ThemeCompileError> {
        let mut error = None;
        let mut check = Check {
            theme: self.id,
            error: &mut error,
        };

        if !self.schema.is_supported() {
            check.fail(ThemeCompileError::UnsupportedSchema {
                theme: self.id,
                found: self.schema,
                supported: ThemeSchemaVersion::CURRENT,
            });
        }
        if self.generation.0 == 0 {
            check.fail(ThemeCompileError::ZeroGeneration { theme: self.id });
        }

        let tokens = &self.tokens;
        for (name, color) in palette_fields(&tokens.palette) {
            check.color(name, color);
        }
        check.color("titlebar", tokens.titlebar_color());
        check.color("foundation.accent.base", tokens.foundation.accent.base);
        check.color("foundation.accent.strong", tokens.foundation.accent.strong);
        check.color(
            "foundation.accent.on_soft",
            tokens.foundation.accent.on_soft,
        );
        check.color("foundation.accent.text", tokens.foundation.accent.text);
        for (index, alpha) in tokens.foundation.accent.alphas().into_iter().enumerate() {
            check.alpha(ACCENT_ALPHA_NAMES[index], alpha);
        }

        for (name, value) in metrics_fields(tokens.metrics) {
            check.length(name, value);
        }
        for (index, step) in tokens.spacing.steps().into_iter().enumerate() {
            check.length(SPACING_NAMES[index], step);
        }
        check.length("border.hairline", tokens.border.hairline);
        for (index, alpha) in tokens.opacity.alphas().into_iter().enumerate() {
            check.alpha(OPACITY_NAMES[index], alpha);
        }

        for (index, size) in self.typography.sizes().into_iter().enumerate() {
            check.font_size(TYPE_SIZE_NAMES[index], size);
        }
        for (index, weight) in self.typography.weights().into_iter().enumerate() {
            check.font_weight(TYPE_WEIGHT_NAMES[index], weight);
        }

        for (index, millis) in self.motion.durations().into_iter().enumerate() {
            check.duration(MOTION_NAMES[index], millis);
        }

        for (name, shadow) in [
            ("effects.surface", self.effects.surface),
            ("effects.overlay", self.effects.overlay),
        ] {
            check.color(name, shadow.color);
            let _ = check.finite(name, shadow.offset_x);
            let _ = check.finite(name, shadow.offset_y);
            check.length(name, shadow.blur_radius);
            let _ = check.finite(name, shadow.spread_radius);
        }

        if let Some(error) = error {
            return Err(error);
        }

        let recipes = self.compile_recipes()?;
        let style_model = StyleModelRef::with_tokens(
            self.mode,
            tokens.metrics,
            tokens.palette,
            tokens.titlebar_color(),
            tokens.opacity,
        );
        Ok(super::CompiledTheme::new(
            ThemeIdentity {
                id: self.id,
                schema: self.schema,
                generation: self.generation,
            },
            style_model,
            tokens.spacing,
            tokens.border,
            self.typography,
            self.motion,
            self.effects,
            self.surfaces,
            recipes,
        ))
    }

    fn compile_recipes(&self) -> Result<CompiledRecipes, ThemeCompileError> {
        let mut families = [ComponentRecipe {
            foreground: SemanticColorRole::Text,
            foreground_checked: None,
        }; ComponentRecipeId::COUNT];
        for id in ComponentRecipeId::ALL {
            let draft = self.components.draft(id);
            let Some(foreground) = draft.foreground else {
                return Err(ThemeCompileError::MissingRecipe {
                    theme: self.id,
                    component: id,
                    slot: "foreground",
                });
            };
            families[id.index()] = ComponentRecipe {
                foreground,
                foreground_checked: draft.foreground_checked,
            };
        }

        let mut variants = [ButtonVariantRecipe {
            foreground: SemanticColorRole::Text,
            background: None,
            border: None,
            hovered_background: SemanticColorRole::Hover,
            pressed_background: SemanticColorRole::Active,
        }; 8];
        for kind in BUTTON_KINDS {
            let draft = self.components.button.variants[button_index(kind)];
            let missing = |slot: &'static str| ThemeCompileError::MissingButtonSlot {
                theme: self.id,
                variant: button_kind_name(kind),
                slot,
            };
            variants[button_index(kind)] = ButtonVariantRecipe {
                foreground: draft.foreground.ok_or_else(|| missing("foreground"))?,
                background: draft.background.ok_or_else(|| missing("background"))?,
                border: draft.border.ok_or_else(|| missing("border"))?,
                hovered_background: draft
                    .hovered_background
                    .ok_or_else(|| missing("hovered_background"))?,
                pressed_background: draft
                    .pressed_background
                    .ok_or_else(|| missing("pressed_background"))?,
            };
        }
        let invalid_border =
            self.components
                .button
                .invalid_border
                .ok_or(ThemeCompileError::MissingButtonSlot {
                    theme: self.id,
                    variant: "*",
                    slot: "invalid_border",
                })?;

        let status = self
            .components
            .status
            .ok_or(ThemeCompileError::MissingStatusRecipe { theme: self.id })?;

        Ok(CompiledRecipes::new(
            families,
            ButtonRecipe::from_variants(variants, invalid_border),
            status,
        ))
    }
}

const ACCENT_ALPHA_NAMES: [&str; 3] = [
    "foundation.accent.soft_alpha",
    "foundation.accent.soft_hover_alpha",
    "foundation.accent.soft_pressed_alpha",
];

const SPACING_NAMES: [&str; 10] = [
    "spacing.xxs",
    "spacing.xs",
    "spacing.sm",
    "spacing.md",
    "spacing.lg",
    "spacing.xl",
    "spacing.xxl",
    "spacing.xxxl",
    "spacing.page_tight",
    "spacing.page",
];

const OPACITY_NAMES: [&str; 5] = [
    "opacity.warning_soft",
    "opacity.warning_soft_hover",
    "opacity.warning_soft_pressed",
    "opacity.danger_soft_hover",
    "opacity.danger_soft_pressed",
];

const TYPE_SIZE_NAMES: [&str; 9] = [
    "typography.hint",
    "typography.meta",
    "typography.body",
    "typography.section",
    "typography.heading",
    "typography.title",
    "typography.display",
    "typography.line",
    "typography.line_tall",
];

const TYPE_WEIGHT_NAMES: [&str; 4] = [
    "typography.regular",
    "typography.medium",
    "typography.semibold",
    "typography.bold",
];

const MOTION_NAMES: [&str; 8] = [
    "motion.hover_color",
    "motion.overlay_fade",
    "motion.menu_opacity",
    "motion.menu_pop",
    "motion.sidebar_collapse",
    "motion.skeleton_pulse",
    "motion.spinner_rotation",
    "motion.loading_spin",
];

fn palette_fields(palette: &SemanticPalette) -> [(&'static str, SemanticColor); 24] {
    [
        ("palette.background", palette.background),
        ("palette.surface", palette.surface),
        ("palette.subtle", palette.subtle),
        ("palette.hover", palette.hover),
        ("palette.active", palette.active),
        ("palette.selected", palette.selected),
        ("palette.selected_hover", palette.selected_hover),
        ("palette.selected_pressed", palette.selected_pressed),
        ("palette.border", palette.border),
        ("palette.border_soft", palette.border_soft),
        ("palette.border_strong", palette.border_strong),
        ("palette.text", palette.text),
        ("palette.muted", palette.muted),
        ("palette.faint", palette.faint),
        ("palette.accent", palette.accent),
        ("palette.accent_strong", palette.accent_strong),
        ("palette.accent_soft", palette.accent_soft),
        ("palette.accent_soft_hover", palette.accent_soft_hover),
        ("palette.accent_soft_pressed", palette.accent_soft_pressed),
        ("palette.accent_on_soft", palette.accent_on_soft),
        ("palette.accent_text", palette.accent_text),
        ("palette.success", palette.success),
        ("palette.warning", palette.warning),
        ("palette.danger", palette.danger),
    ]
}

fn metrics_fields(metrics: ThemeMetrics) -> [(&'static str, f32); 23] {
    [
        ("metrics.radius_xs", metrics.radius_xs),
        ("metrics.radius_sm", metrics.radius_sm),
        ("metrics.radius_md", metrics.radius_md),
        ("metrics.radius_lg", metrics.radius_lg),
        ("metrics.radius_xl", metrics.radius_xl),
        (
            "metrics.compact_control_height",
            metrics.compact_control_height,
        ),
        ("metrics.control_height", metrics.control_height),
        (
            "metrics.compact_control_padding_x",
            metrics.compact_control_padding_x,
        ),
        ("metrics.control_padding_x", metrics.control_padding_x),
        ("metrics.selection_height", metrics.selection_height),
        ("metrics.icon_button_size", metrics.icon_button_size),
        ("metrics.panel_padding_x", metrics.panel_padding_x),
        ("metrics.panel_padding_y", metrics.panel_padding_y),
        ("metrics.field_padding_x", metrics.field_padding_x),
        ("metrics.list_item_padding_x", metrics.list_item_padding_x),
        (
            "metrics.large_control_padding_x",
            metrics.large_control_padding_x,
        ),
        ("metrics.scrollbar.thickness", metrics.scrollbar.thickness),
        (
            "metrics.scrollbar.thumb_thickness",
            metrics.scrollbar.thumb_thickness,
        ),
        (
            "metrics.scrollbar.thumb_min_length",
            metrics.scrollbar.thumb_min_length,
        ),
        ("metrics.switch.track_width", metrics.switch.track_width),
        ("metrics.switch.track_height", metrics.switch.track_height),
        ("metrics.switch.label_gap", metrics.switch.label_gap),
        (
            "metrics.scrollbar.track_inset",
            metrics.scrollbar.track_inset,
        ),
    ]
}

/// The recipe table both built-ins share.
///
/// Light and dark disagree about colour *values*, not about which role a
/// family paints with — that is the whole point of a semantic layer. A theme
/// that does want a different mapping overrides the entry; it does not fork
/// the table.
const fn builtin_registry() -> ComponentThemeRegistry {
    use ComponentRecipeId as Id;
    use SemanticColorRole as Role;

    ComponentThemeRegistry {
        families: {
            let mut families = ComponentThemeRegistry::empty().families;
            families[Id::Button.index()] = ComponentRecipeDraft::plain(Role::Text);
            families[Id::Icon.index()] = ComponentRecipeDraft::plain(Role::Muted);
            families[Id::TextInput.index()] = ComponentRecipeDraft::plain(Role::Text);
            families[Id::Checkbox.index()] =
                ComponentRecipeDraft::checkable(Role::Muted, Role::AccentText);
            families[Id::Switch.index()] =
                ComponentRecipeDraft::checkable(Role::Muted, Role::AccentText);
            families[Id::Range.index()] = ComponentRecipeDraft::plain(Role::Accent);
            families[Id::Card.index()] = ComponentRecipeDraft::plain(Role::Accent);
            families[Id::ListItem.index()] = ComponentRecipeDraft::plain(Role::Accent);
            families[Id::Selection.index()] = ComponentRecipeDraft::plain(Role::Text);
            families[Id::Menu.index()] = ComponentRecipeDraft::plain(Role::Text);
            families[Id::Overlay.index()] = ComponentRecipeDraft::plain(Role::Text);
            families[Id::Scrollbar.index()] = ComponentRecipeDraft::plain(Role::BorderStrong);
            families[Id::Indicator.index()] = ComponentRecipeDraft::plain(Role::Accent);
            families[Id::Content.index()] = ComponentRecipeDraft::plain(Role::Text);
            families
        },
        button: builtin_button_recipe(),
        status: Some(StatusRecipe::DEFAULT),
    }
}

const fn builtin_button_recipe() -> ButtonRecipeDraft {
    use crate::semantics::ButtonKind as Kind;
    use SemanticColorRole as Role;

    const fn variant(
        foreground: SemanticColorRole,
        background: Option<SemanticColorRole>,
        border: Option<SemanticColorRole>,
        hovered_background: SemanticColorRole,
        pressed_background: SemanticColorRole,
    ) -> ButtonVariantDraft {
        ButtonVariantDraft {
            foreground: Some(foreground),
            background: Some(background),
            border: Some(border),
            hovered_background: Some(hovered_background),
            pressed_background: Some(pressed_background),
        }
    }

    let mut draft = ButtonRecipeDraft::empty();
    draft = draft.with(
        Kind::Ghost,
        variant(Role::Text, None, None, Role::Hover, Role::Active),
    );
    draft = draft.with(
        Kind::Subtle,
        variant(
            Role::Text,
            Some(Role::Subtle),
            Some(Role::BorderSoft),
            Role::Hover,
            Role::Active,
        ),
    );
    draft = draft.with(
        Kind::Selected,
        variant(
            Role::Text,
            Some(Role::Selected),
            None,
            Role::SelectedHover,
            Role::SelectedPressed,
        ),
    );
    draft = draft.with(
        Kind::Primary,
        variant(
            Role::AccentOnSoft,
            Some(Role::AccentSoft),
            None,
            Role::AccentSoftHover,
            Role::AccentSoftPressed,
        ),
    );
    draft = draft.with(
        Kind::Warning,
        variant(
            Role::Warning,
            Some(Role::WarningSoft),
            None,
            Role::WarningSoftHover,
            Role::WarningSoftPressed,
        ),
    );
    draft = draft.with(
        Kind::Danger,
        variant(
            Role::Danger,
            None,
            None,
            Role::DangerSoftHover,
            Role::DangerSoftPressed,
        ),
    );
    draft = draft.with(
        Kind::Text,
        variant(Role::Accent, None, None, Role::Hover, Role::Active),
    );
    draft = draft.with(
        Kind::Menu,
        variant(
            Role::Text,
            Some(Role::Subtle),
            Some(Role::BorderSoft),
            Role::Hover,
            Role::Active,
        ),
    );
    draft.invalid_border = Some(Role::Danger);
    draft
}

impl ThemeDefinition {
    /// The built-in dark theme.
    pub const NANA_DARK: Self = Self::builtin(ThemeId::new("nana.dark"), ThemeMode::Dark);
    /// The built-in light theme.
    pub const NANA_LIGHT: Self = Self::builtin(ThemeId::new("nana.light"), ThemeMode::Light);

    const fn builtin(id: ThemeId, mode: ThemeMode) -> Self {
        Self {
            id,
            schema: ThemeSchemaVersion::CURRENT,
            generation: ThemeGeneration::FIRST,
            mode,
            tokens: DesignTokens::for_mode(mode),
            typography: TypographyTokens::DEFAULT,
            motion: MotionTokens::DEFAULT,
            effects: EffectTokens::for_mode(mode),
            surfaces: SurfaceTokens::DEFAULT,
            components: builtin_registry(),
        }
    }

    /// The built-in definition for `mode`.
    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::NANA_DARK,
            ThemeMode::Light => Self::NANA_LIGHT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::ButtonKind;
    use crate::theme::tokens::{ElevationRole, MotionRole};

    fn dark() -> ThemeDefinition {
        ThemeDefinition::NANA_DARK
    }

    /// The `expect` in `builtin_theme` is only safe because of this.
    #[test]
    fn both_built_in_themes_compile() {
        for definition in [ThemeDefinition::NANA_DARK, ThemeDefinition::NANA_LIGHT] {
            definition
                .compile()
                .unwrap_or_else(|error| panic!("{} must compile: {error}", definition.id));
        }
    }

    /// Issue #102 acceptance: NanaLight / NanaDark migrate **completely**, and
    /// the Gallery baseline sees no unexpected change.
    ///
    /// The second half is what this asserts, and it asserts it by equality
    /// rather than by re-recording pixels: if compiling the definition
    /// produces exactly the palette and metrics the tree already renders with,
    /// no fixture can move. A snapshot run then has nothing to prove, which is
    /// the point — a migration that needs 615 images re-blessed to show it is
    /// safe has not been shown to be safe.
    #[test]
    fn the_built_in_definitions_compile_to_exactly_the_tokens_already_rendered() {
        for (definition, palette, mode) in [
            (
                ThemeDefinition::NANA_DARK,
                SemanticPalette::dark(),
                ThemeMode::Dark,
            ),
            (
                ThemeDefinition::NANA_LIGHT,
                SemanticPalette::light(),
                ThemeMode::Light,
            ),
        ] {
            let compiled = definition.compile().expect("compiles");
            assert_eq!(compiled.mode(), mode);
            assert_eq!(compiled.palette(), palette, "{} palette", definition.id);
            assert_eq!(compiled.metrics(), UI_METRICS, "{} metrics", definition.id);
            assert_eq!(
                compiled.style_model().titlebar,
                palette.surface,
                "{} chrome follows Surface unless pinned",
                definition.id
            );
            assert_eq!(
                compiled.duration(MotionRole::HoverColor),
                crate::motion::HOVER_COLOR
            );
            assert_eq!(
                compiled.shadow(ElevationRole::Surface).blur_radius,
                if mode == ThemeMode::Dark { 30.0 } else { 26.0 }
            );
        }
    }

    /// Issue #102 acceptance: a theme has an id, a schema and a generation.
    #[test]
    fn a_theme_carries_identity_schema_and_generation() {
        let compiled = dark().compile().expect("compiles");
        let identity = compiled.identity();
        assert_eq!(identity.id.as_str(), "nana.dark");
        assert_eq!(identity.schema, ThemeSchemaVersion::CURRENT);
        assert_eq!(identity.generation, ThemeGeneration::FIRST);
        assert_ne!(
            ThemeDefinition::NANA_LIGHT.id,
            ThemeDefinition::NANA_DARK.id,
            "two themes are two identities"
        );
    }

    /// The generation policy is "bump on any token change", and it is only a
    /// policy if the mutators honour it.
    #[test]
    fn every_token_change_bumps_the_generation() {
        let base = dark();
        let mut metrics = UI_METRICS;
        metrics.radius_md += 1.0;
        for changed in [
            base.with_metrics(metrics),
            base.with_palette(SemanticPalette::light()),
            base.with_titlebar(Some(SemanticColor::rgb8(1, 2, 3))),
            base.with_accent(AccentRamp::LIGHT),
        ] {
            assert_eq!(
                changed.generation,
                base.generation.next(),
                "a token change must advance the revision"
            );
        }
        // Re-identifying a derived theme restarts its own history rather than
        // continuing someone else's.
        let derived = base
            .with_metrics(metrics)
            .with_id(ThemeId::new("acme.dark"));
        assert_eq!(derived.generation, ThemeGeneration::FIRST);
        assert_eq!(derived.id.as_str(), "acme.dark");
    }

    /// The foundation layer's reason to exist. Setting `palette.accent` alone
    /// leaves the three soft fills on the old hue; going through the ramp does
    /// not. This is the bug the accent-only theme path had.
    #[test]
    fn re_accenting_through_the_ramp_moves_the_whole_accent_family() {
        let coral = AccentRamp {
            base: SemanticColor::rgb8(240, 145, 123),
            ..AccentRamp::DARK
        };
        let compiled = dark().with_accent(coral).compile().expect("compiles");
        let palette = compiled.palette();
        assert_eq!(palette.accent, coral.base);
        for (name, soft, alpha) in [
            ("accent_soft", palette.accent_soft, coral.soft_alpha),
            (
                "accent_soft_hover",
                palette.accent_soft_hover,
                coral.soft_hover_alpha,
            ),
            (
                "accent_soft_pressed",
                palette.accent_soft_pressed,
                coral.soft_pressed_alpha,
            ),
        ] {
            assert_eq!(
                (soft.r, soft.g, soft.b),
                (coral.base.r, coral.base.g, coral.base.b),
                "{name} must follow the base hue"
            );
            assert_eq!(soft.a, alpha, "{name} keeps its own alpha");
        }

        // The contrast pair is a design decision, not a formula, so it stays
        // where the author left it.
        assert_eq!(palette.accent_text, AccentRamp::DARK.text);

        // Writing the palette directly is still allowed and still literal: the
        // ramp records what the palette says, it does not repair it.
        let mut stale = SemanticPalette::dark();
        stale.accent = coral.base;
        let recorded = dark().with_palette(stale);
        assert_eq!(recorded.tokens.palette, stale);
        assert_eq!(recorded.tokens.foundation.accent.base, coral.base);
    }

    /// Issue #101 §1.4 called the soft state layers out as literals inside
    /// `SemanticPalette::get`, with light and dark told apart by sniffing
    /// `background.r > 0.5`. They are tokens now, so a theme can move them and
    /// a mid-grey background cannot pick the wrong branch.
    #[test]
    fn the_soft_state_layers_come_from_the_theme_not_from_background_brightness() {
        let dark_model = ThemeDefinition::NANA_DARK
            .compile()
            .expect("compiles")
            .style_model();
        let light_model = ThemeDefinition::NANA_LIGHT
            .compile()
            .expect("compiles")
            .style_model();
        assert_eq!(
            dark_model.color(SemanticColorRole::WarningSoft).a,
            OpacityTokens::DARK.warning_soft
        );
        assert_eq!(
            light_model.color(SemanticColorRole::WarningSoft).a,
            OpacityTokens::LIGHT.warning_soft
        );

        // A dark theme whose background happens to be bright used to flip to
        // the light alpha. It now keeps its own.
        let mut bright = SemanticPalette::dark();
        bright.background = SemanticColor::rgb8(200, 200, 200);
        let model = ThemeDefinition::NANA_DARK
            .with_palette(bright)
            .compile()
            .expect("compiles")
            .style_model();
        assert_eq!(
            model.color(SemanticColorRole::WarningSoft).a,
            OpacityTokens::DARK.warning_soft
        );
    }

    /// Fail-closed, one case per validation rule. Each starts from a theme
    /// that compiles, so the rejection is attributable to the one field moved.
    #[test]
    fn an_invalid_token_is_rejected_rather_than_installed() {
        let mut negative = UI_METRICS;
        negative.radius_md = -1.0;
        assert!(matches!(
            dark().with_metrics(negative).compile(),
            Err(ThemeCompileError::NegativeLength { token, .. }) if token == "metrics.radius_md"
        ));

        let mut nan = UI_METRICS;
        nan.control_height = f32::NAN;
        assert!(matches!(
            dark().with_metrics(nan).compile(),
            Err(ThemeCompileError::NotFinite { token, .. }) if token == "metrics.control_height"
        ));

        let mut washed = dark();
        washed.tokens.opacity.warning_soft = 1.5;
        assert!(matches!(
            washed.compile(),
            Err(ThemeCompileError::AlphaOutOfRange { token, .. })
                if token == "opacity.warning_soft"
        ));

        let mut tiny = dark();
        tiny.typography.body = 0.0;
        assert!(matches!(
            tiny.compile(),
            Err(ThemeCompileError::FontSizeOutOfRange { token, .. })
                if token == "typography.body"
        ));

        let mut heavy = dark();
        heavy.typography.bold = 1200;
        assert!(matches!(
            heavy.compile(),
            Err(ThemeCompileError::FontWeightOutOfRange { token, .. })
                if token == "typography.bold"
        ));

        let mut frozen = dark();
        frozen.motion.hover_color_ms = 0;
        assert!(matches!(
            frozen.compile(),
            Err(ThemeCompileError::ZeroDuration { token, .. }) if token == "motion.hover_color"
        ));

        let mut ghost_color = dark();
        ghost_color.tokens.palette.text = SemanticColor::rgba(2.0, 0.0, 0.0, 1.0);
        assert!(matches!(
            ghost_color.compile(),
            Err(ThemeCompileError::AlphaOutOfRange { token, .. }) if token == "palette.text"
        ));

        let mut ancient = dark();
        ancient.schema = ThemeSchemaVersion::new(0, 9);
        assert!(matches!(
            ancient.compile(),
            Err(ThemeCompileError::UnsupportedSchema { .. })
        ));

        let mut unversioned = dark();
        unversioned.generation = ThemeGeneration(0);
        assert!(matches!(
            unversioned.compile(),
            Err(ThemeCompileError::ZeroGeneration { .. })
        ));
    }

    /// Issue #102 §6: a recipe that references a token it never set fails
    /// closed. With typed roles there is no "unknown token name" to reference,
    /// so the failure mode that remains is an unset slot — and an unset slot
    /// must not quietly become `Text`.
    #[test]
    fn a_recipe_with_an_unset_slot_fails_closed() {
        let mut forgot_family = dark();
        forgot_family.components.families[ComponentRecipeId::Scrollbar.index()] =
            ComponentRecipeDraft::default();
        assert!(matches!(
            forgot_family.compile(),
            Err(ThemeCompileError::MissingRecipe {
                component: ComponentRecipeId::Scrollbar,
                slot: "foreground",
                ..
            })
        ));

        let mut forgot_variant = dark();
        forgot_variant.components.button.variants[button_index(ButtonKind::Primary)]
            .hovered_background = None;
        assert!(matches!(
            forgot_variant.compile(),
            Err(ThemeCompileError::MissingButtonSlot {
                variant: "Primary",
                slot: "hovered_background",
                ..
            })
        ));

        let mut forgot_status = dark();
        forgot_status.components.status = None;
        assert!(matches!(
            forgot_status.compile(),
            Err(ThemeCompileError::MissingStatusRecipe { .. })
        ));

        // An empty registry is rejected on its first family, not accepted with
        // defaults filled in behind the author's back.
        let mut empty = dark();
        empty.components = ComponentThemeRegistry::empty();
        assert!(matches!(
            empty.compile(),
            Err(ThemeCompileError::MissingRecipe { .. })
        ));
    }

    /// The button table moved out of `Button::project` verbatim. This pins the
    /// two variants whose slots differ most, so a "tidy-up" of the table has
    /// to admit it is changing the design.
    #[test]
    fn the_button_recipe_still_says_what_the_component_used_to_say() {
        let compiled = dark().compile().expect("compiles");
        let primary = compiled.recipes().button().variant(ButtonKind::Primary);
        assert_eq!(primary.foreground, SemanticColorRole::AccentOnSoft);
        assert_eq!(primary.background, Some(SemanticColorRole::AccentSoft));
        assert_eq!(primary.border, None);
        assert_eq!(
            primary.hovered_background,
            SemanticColorRole::AccentSoftHover
        );
        assert_eq!(
            primary.pressed_background,
            SemanticColorRole::AccentSoftPressed
        );

        let ghost = compiled.recipes().button().variant(ButtonKind::Ghost);
        assert_eq!(ghost.foreground, SemanticColorRole::Text);
        assert_eq!(ghost.background, None, "a ghost button has no fill");
        assert_eq!(ghost.hovered_background, SemanticColorRole::Hover);

        assert_eq!(
            compiled.recipes().button().invalid_border,
            SemanticColorRole::Danger
        );
    }

    /// A family with no indicator answers the rest colour when asked for a
    /// checked one: the caller is a paint path and has to get a colour.
    #[test]
    fn asking_an_indicator_less_family_for_its_checked_colour_falls_back() {
        let compiled = dark().compile().expect("compiles");
        let recipes = compiled.recipes();
        assert_eq!(
            recipes.foreground(ComponentRecipeId::Checkbox, true),
            SemanticColorRole::AccentText
        );
        assert_eq!(
            recipes.foreground(ComponentRecipeId::Checkbox, false),
            SemanticColorRole::Muted
        );
        assert_eq!(
            recipes.foreground(ComponentRecipeId::Menu, true),
            recipes.foreground(ComponentRecipeId::Menu, false)
        );
    }

    /// Identity is the cheap question and it is allowed to be wrong about
    /// values; installs compare values. Both halves are load-bearing, so both
    /// are pinned.
    #[test]
    fn identity_answers_which_revision_and_values_answer_what_changed() {
        let base = dark().compile().expect("compiles");
        let same = dark().compile().expect("compiles");
        assert!(base.is_same_revision(&same));
        assert_eq!(base, same);

        let mut metrics = UI_METRICS;
        metrics.radius_md += 1.0;
        let moved = dark().with_metrics(metrics).compile().expect("compiles");
        assert!(!base.is_same_revision(&moved));
        assert_ne!(base, moved);

        // A theme that changed a value without bumping is a bug in the theme.
        // `is_same_revision` believes it; equality does not, which is why the
        // install path uses equality.
        let mut liar = dark();
        liar.tokens.metrics = metrics;
        let liar = liar.compile().expect("compiles");
        assert!(base.is_same_revision(&liar), "identity trusts the author");
        assert_ne!(base, liar, "values do not");
    }
}
