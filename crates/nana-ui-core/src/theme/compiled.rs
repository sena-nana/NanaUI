//! [`CompiledTheme`]: the validated, runtime-facing form of a theme.
//!
//! Issue #102 §5 asks the compiled representation for typed indices, validated
//! references, precomputed recipe lookup, a generation and dependency
//! metadata. Concretely, here that means:
//!
//! - **Typed indices.** Every lookup is `enum -> array index` or a `match`.
//!   There is no map and no string on this side of
//!   [`ThemeDefinition::compile`](super::ThemeDefinition::compile).
//! - **Validated references.** Recipe slots arrive as `Option` and leave as
//!   values; a definition that left one unset never becomes a `CompiledTheme`.
//! - **Precomputed lookup.** [`CompiledRecipes`](super::CompiledRecipes) is a
//!   flat array, not a walk over an authoring tree.
//! - **Generation.** [`Self::identity`] carries it, so "same theme?" is one
//!   comparison rather than a token-by-token diff.
//!
//! What is deliberately **not** precomputed is a 39-entry resolved colour
//! table. Resolving a [`SemanticColorRole`](crate::SemanticColorRole) is
//! already a `match` plus at most an alpha substitution; a table would add
//! ~600 bytes to a structure the runtime clones per theme install and would
//! buy nothing. Issue #101 §1.5 paid for that lesson once with
//! `LayoutStyle`; the rule it left behind is to size the cache to the cost it
//! removes.

use super::definition::ThemeIdentity;
use super::recipe::CompiledRecipes;
use super::tokens::{
    BorderTokens, EffectTokens, ElevationRole, MotionRole, MotionTokens, ShadowToken,
    SpacingTokens, SurfaceTokens, TypographyTokens,
};
use super::{ChromeRadii, ThemeDefinition, ThemeMetrics, ThemeMode};
use crate::style_model::{SemanticPalette, StyleModelRef};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// A validated theme, ready to install.
///
/// Held behind an `Arc` by the runtime rather than copied: it is ~1 KB, and a
/// `Copy` structure that size on a read path is exactly the mistake Issue #101
/// §1.5 measured. The small, hot slice — mode, palette, metrics, chrome colour
/// and state-layer alphas — stays available as [`Self::style_model`], which is
/// the handle per-node style resolution already takes.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTheme {
    identity: ThemeIdentity,
    style_model: StyleModelRef,
    spacing: SpacingTokens,
    border: BorderTokens,
    typography: TypographyTokens,
    motion: MotionTokens,
    effects: EffectTokens,
    surfaces: SurfaceTokens,
    recipes: CompiledRecipes,
}

impl CompiledTheme {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        identity: ThemeIdentity,
        style_model: StyleModelRef,
        spacing: SpacingTokens,
        border: BorderTokens,
        typography: TypographyTokens,
        motion: MotionTokens,
        effects: EffectTokens,
        surfaces: SurfaceTokens,
        recipes: CompiledRecipes,
    ) -> Self {
        Self {
            identity,
            style_model,
            spacing,
            border,
            typography,
            motion,
            effects,
            surfaces,
            recipes,
        }
    }

    pub const fn identity(&self) -> ThemeIdentity {
        self.identity
    }

    /// The hot-path token slice: mode, metrics, palette, chrome, state layers.
    pub const fn style_model(&self) -> StyleModelRef {
        self.style_model
    }

    pub const fn mode(&self) -> ThemeMode {
        self.style_model.theme_mode
    }

    pub const fn palette(&self) -> SemanticPalette {
        self.style_model.palette
    }

    pub const fn metrics(&self) -> ThemeMetrics {
        self.style_model.metrics
    }

    pub const fn spacing(&self) -> SpacingTokens {
        self.spacing
    }

    pub const fn border(&self) -> BorderTokens {
        self.border
    }

    pub const fn typography(&self) -> TypographyTokens {
        self.typography
    }

    pub const fn motion(&self) -> MotionTokens {
        self.motion
    }

    pub const fn effects(&self) -> EffectTokens {
        self.effects
    }

    pub const fn surfaces(&self) -> SurfaceTokens {
        self.surfaces
    }

    pub const fn recipes(&self) -> &CompiledRecipes {
        &self.recipes
    }

    /// The four radius steps, resolved, for the renderer to consume.
    pub fn chrome_radii(&self) -> ChromeRadii {
        ChromeRadii::from(self.metrics())
    }

    pub const fn duration(&self, role: MotionRole) -> Duration {
        self.motion.duration(role)
    }

    pub const fn shadow(&self, role: ElevationRole) -> ShadowToken {
        self.effects.shadow(role)
    }

    /// Whether `other` is the same theme at the same revision.
    ///
    /// This is the cheap question. It is **not** a substitute for comparing
    /// values when correctness depends on them: a theme that changed a token
    /// without bumping its generation is a bug in that theme, and this method
    /// will happily call it unchanged. Install compares values; caches and
    /// diagnostics compare identity.
    pub fn is_same_revision(&self, other: &Self) -> bool {
        self.identity.id == other.identity.id
            && self.identity.generation == other.identity.generation
    }
}

static NANA_DARK: LazyLock<Arc<CompiledTheme>> = LazyLock::new(|| {
    Arc::new(
        ThemeDefinition::NANA_DARK
            .compile()
            .expect("the built-in dark theme compiles"),
    )
});

static NANA_LIGHT: LazyLock<Arc<CompiledTheme>> = LazyLock::new(|| {
    Arc::new(
        ThemeDefinition::NANA_LIGHT
            .compile()
            .expect("the built-in light theme compiles"),
    )
});

fn builtin_slot(mode: ThemeMode) -> &'static Arc<CompiledTheme> {
    match mode {
        ThemeMode::Dark => &NANA_DARK,
        ThemeMode::Light => &NANA_LIGHT,
    }
}

/// The compiled built-in theme for `mode`.
///
/// Compiled once per process. The `expect` is not optimism: both definitions
/// are compiled by a unit test, so a change that would panic here fails the
/// test suite first.
pub fn builtin_theme(mode: ThemeMode) -> &'static CompiledTheme {
    builtin_slot(mode)
}

/// The same theme, shareable. Installing a built-in mode clones this handle
/// rather than recompiling — the runtime holds its theme behind an `Arc`
/// precisely so switching modes costs a refcount, not a rebuild.
pub fn builtin_theme_arc(mode: ThemeMode) -> Arc<CompiledTheme> {
    Arc::clone(builtin_slot(mode))
}

impl Default for CompiledTheme {
    fn default() -> Self {
        builtin_theme(ThemeMode::default()).clone()
    }
}
