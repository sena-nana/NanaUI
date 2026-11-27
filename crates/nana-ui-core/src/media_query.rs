//! Backend-neutral `@media` query contract with Kleene 3-valued evaluation.
//!
//! No CSS parsing lives here: `nana-ui-css` lowers `@media` preludes into
//! [`MediaQuery`] / [`MediaCondition`] / [`MediaFeature`] values, and Runtime
//! or build-time compilers evaluate them against a [`MediaEnv`].
//!
//! Evaluation follows CSS Media Queries Level 4 strong Kleene 3-valued logic:
//! - `Some(true)` — definitely true
//! - `Some(false)` — definitely false
//! - `None` — unknown (e.g. an unrecognized media feature)
//!
//! Truth tables:
//! - `Not(None) == None`
//! - `And(Some(false), None) == Some(false)`
//! - `Or(Some(true), None) == Some(true)`
//! - [`MediaQuery::matches`] returns `self.eval_tri(env) == Some(true)`.

use serde::{Deserialize, Serialize};

/// Kleene strong 3-valued logical NOT.
#[inline]
pub const fn kleene_not(value: Option<bool>) -> Option<bool> {
    match value {
        Some(b) => Some(!b),
        None => None,
    }
}

/// Kleene strong 3-valued logical AND.
#[inline]
pub const fn kleene_and(lhs: Option<bool>, rhs: Option<bool>) -> Option<bool> {
    match (lhs, rhs) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// Kleene strong 3-valued logical OR.
#[inline]
pub const fn kleene_or(lhs: Option<bool>, rhs: Option<bool>) -> Option<bool> {
    match (lhs, rhs) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// CSS media type (`all` / `screen` / `print` / unrecognized).
///
/// Per CSS Media Queries Level 4 §3.1, an unrecognized media type (`Other`)
/// evaluates to `Some(false)` (never `None` / unknown), so `not other` matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaType {
    #[default]
    All,
    Screen,
    Print,
    Other,
}

impl MediaType {
    /// Whether a query's media type matches the environment's device type.
    #[inline]
    pub const fn matches_env_type(self, env_type: MediaType) -> bool {
        match self {
            Self::All => true,
            Self::Screen => matches!(env_type, Self::Screen | Self::All),
            Self::Print => matches!(env_type, Self::Print | Self::All),
            Self::Other => false,
        }
    }
}

/// Relational comparison operator for range-capable media features (`width`,
/// `height`, `aspect-ratio`, `resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaComparison {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl MediaComparison {
    #[inline]
    fn compare_f32(self, lhs: f32, rhs: f32) -> bool {
        if !lhs.is_finite() || !rhs.is_finite() {
            return false;
        }
        match self {
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Eq => (lhs - rhs).abs() <= 1e-4,
            Self::Ge => lhs >= rhs,
            Self::Gt => lhs > rhs,
        }
    }

    #[inline]
    fn compare_ratio(self, env_w: f32, env_h: f32, target_w: f32, target_h: f32) -> bool {
        if !env_w.is_finite()
            || !env_h.is_finite()
            || !target_w.is_finite()
            || !target_h.is_finite()
            || env_w < 0.0
            || env_h <= 0.0
            || target_w < 0.0
            || target_h <= 0.0
        {
            return false;
        }
        // Cross-multiply so integer ratios like `16/9` at `1600x900` compare exactly.
        let lhs = env_w * target_h;
        let rhs = env_h * target_w;
        match self {
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Eq => {
                let scale = lhs.abs().max(rhs.abs()).max(1.0);
                (lhs - rhs).abs() <= 1e-4 * scale
            }
            Self::Ge => lhs >= rhs,
            Self::Gt => lhs > rhs,
        }
    }
}

/// CSS `orientation` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaOrientation {
    Portrait,
    Landscape,
}

/// CSS `prefers-color-scheme` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaColorScheme {
    #[default]
    Light,
    Dark,
}

/// CSS `prefers-reduced-motion` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaReducedMotion {
    #[default]
    NoPreference,
    Reduce,
}

/// CSS `prefers-contrast` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaContrast {
    #[default]
    NoPreference,
    More,
    Less,
    Custom,
}

/// CSS `hover` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaHover {
    None,
    #[default]
    Hover,
}

/// CSS `pointer` media feature value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaPointer {
    None,
    Coarse,
    #[default]
    Fine,
}

/// Runtime / host environment facts evaluated against [`MediaQuery`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MediaEnv {
    /// Active media type (`Screen` for interactive windows, `Print` for print).
    pub media_type: MediaType,
    /// Viewport width in logical CSS pixels.
    pub width: f32,
    /// Viewport height in logical CSS pixels.
    pub height: f32,
    /// Active system/app color scheme preference.
    pub color_scheme: MediaColorScheme,
    /// Reduced-motion accessibility preference.
    pub reduced_motion: MediaReducedMotion,
    /// Contrast accessibility preference.
    pub contrast: MediaContrast,
    /// Primary input mechanism hover capability.
    pub hover: MediaHover,
    /// Primary input mechanism pointer precision.
    pub pointer: MediaPointer,
    /// Device pixel ratio in `dppx` (`1.0` = 96 dpi, `2.0` = 192 dpi).
    pub resolution_dppx: f32,
}

impl Default for MediaEnv {
    fn default() -> Self {
        Self {
            media_type: MediaType::Screen,
            width: 960.0,
            height: 640.0,
            color_scheme: MediaColorScheme::Light,
            reduced_motion: MediaReducedMotion::NoPreference,
            contrast: MediaContrast::NoPreference,
            hover: MediaHover::Hover,
            pointer: MediaPointer::Fine,
            resolution_dppx: 1.0,
        }
    }
}

impl MediaEnv {
    /// Construct a desktop screen environment with the given logical viewport
    /// size and light/dark scheme, leaving other capabilities at desktop defaults.
    pub fn desktop(width: f32, height: f32, dark: bool) -> Self {
        Self {
            width,
            height,
            color_scheme: if dark {
                MediaColorScheme::Dark
            } else {
                MediaColorScheme::Light
            },
            ..Self::default()
        }
    }

    /// Resolved CSS `orientation`: `Portrait` when `height >= width`,
    /// `Landscape` otherwise (CSS Media Queries Level 4 §4.3).
    #[inline]
    pub fn orientation(&self) -> MediaOrientation {
        if self.height >= self.width {
            MediaOrientation::Portrait
        } else {
            MediaOrientation::Landscape
        }
    }
}

/// One typed media feature predicate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MediaFeature {
    /// `width` / `min-width` / `max-width` / range comparison, in logical px.
    Width(MediaComparison, f32),
    /// `height` / `min-height` / `max-height` / range comparison, in logical px.
    Height(MediaComparison, f32),
    /// `aspect-ratio` / `min-aspect-ratio` / `max-aspect-ratio` / range comparison
    /// expressed as `(cmp, numerator, denominator)`.
    AspectRatio(MediaComparison, f32, f32),
    /// `orientation: portrait | landscape`.
    Orientation(MediaOrientation),
    /// `prefers-color-scheme: light | dark`.
    PrefersColorScheme(MediaColorScheme),
    /// `prefers-reduced-motion: no-preference | reduce`.
    PrefersReducedMotion(MediaReducedMotion),
    /// `prefers-contrast: no-preference | more | less | custom`.
    PrefersContrast(MediaContrast),
    /// `hover: none | hover`.
    Hover(MediaHover),
    /// `pointer: none | coarse | fine`.
    Pointer(MediaPointer),
    /// `resolution` / `min-resolution` / `max-resolution` / range comparison, in `dppx`.
    Resolution(MediaComparison, f32),
    /// Unrecognized or invalid media feature: evaluates to `None` (unknown)
    /// under Kleene 3-valued logic.
    Unknown,
}

impl MediaFeature {
    #[inline]
    pub const fn min_width(px: f32) -> Self {
        Self::Width(MediaComparison::Ge, px)
    }

    #[inline]
    pub const fn max_width(px: f32) -> Self {
        Self::Width(MediaComparison::Le, px)
    }

    #[inline]
    pub const fn width(px: f32) -> Self {
        Self::Width(MediaComparison::Eq, px)
    }

    #[inline]
    pub const fn min_height(px: f32) -> Self {
        Self::Height(MediaComparison::Ge, px)
    }

    #[inline]
    pub const fn max_height(px: f32) -> Self {
        Self::Height(MediaComparison::Le, px)
    }

    #[inline]
    pub const fn height(px: f32) -> Self {
        Self::Height(MediaComparison::Eq, px)
    }

    #[inline]
    pub const fn aspect_ratio(width: f32, height: f32) -> Self {
        Self::AspectRatio(MediaComparison::Eq, width, height)
    }

    #[inline]
    pub const fn min_aspect_ratio(width: f32, height: f32) -> Self {
        Self::AspectRatio(MediaComparison::Ge, width, height)
    }

    #[inline]
    pub const fn max_aspect_ratio(width: f32, height: f32) -> Self {
        Self::AspectRatio(MediaComparison::Le, width, height)
    }

    #[inline]
    pub const fn resolution(dppx: f32) -> Self {
        Self::Resolution(MediaComparison::Eq, dppx)
    }

    #[inline]
    pub const fn min_resolution(dppx: f32) -> Self {
        Self::Resolution(MediaComparison::Ge, dppx)
    }

    #[inline]
    pub const fn max_resolution(dppx: f32) -> Self {
        Self::Resolution(MediaComparison::Le, dppx)
    }

    /// Evaluate this feature under Kleene 3-valued logic.
    ///
    /// Returns `None` for [`Self::Unknown`].
    pub fn eval_tri(&self, env: &MediaEnv) -> Option<bool> {
        match *self {
            Self::Width(cmp, px) => Some(cmp.compare_f32(env.width, px)),
            Self::Height(cmp, px) => Some(cmp.compare_f32(env.height, px)),
            Self::AspectRatio(cmp, w, h) => Some(cmp.compare_ratio(env.width, env.height, w, h)),
            Self::Orientation(orientation) => Some(env.orientation() == orientation),
            Self::PrefersColorScheme(scheme) => Some(env.color_scheme == scheme),
            Self::PrefersReducedMotion(motion) => Some(env.reduced_motion == motion),
            Self::PrefersContrast(contrast) => Some(env.contrast == contrast),
            Self::Hover(hover) => Some(env.hover == hover),
            Self::Pointer(pointer) => Some(env.pointer == pointer),
            Self::Resolution(cmp, dppx) => Some(cmp.compare_f32(env.resolution_dppx, dppx)),
            Self::Unknown => None,
        }
    }

    /// Convenience boolean match (`self.eval_tri(env) == Some(true)`).
    #[inline]
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.eval_tri(env) == Some(true)
    }
}

/// Boolean tree of media feature conditions (`and`, `or`, `not`, feature, `unknown`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaCondition {
    Feature(MediaFeature),
    Not(Box<MediaCondition>),
    And(Vec<MediaCondition>),
    Or(Vec<MediaCondition>),
    /// Unrecognized condition node: evaluates to `None` (unknown).
    Unknown,
}

impl MediaCondition {
    /// Wrap a single [`MediaFeature`] as a condition.
    pub const fn feature(feature: MediaFeature) -> Self {
        Self::Feature(feature)
    }

    /// Negate a condition (`not (...)`).
    pub fn negate(inner: MediaCondition) -> Self {
        Self::Not(Box::new(inner))
    }

    /// Conjunction of conditions (`a and b and ...`).
    pub fn and(conditions: Vec<MediaCondition>) -> Self {
        Self::And(conditions)
    }

    /// Disjunction of conditions (`a or b or ...`).
    pub fn or(conditions: Vec<MediaCondition>) -> Self {
        Self::Or(conditions)
    }

    /// Evaluate under strict Kleene 3-valued logic.
    pub fn eval_tri(&self, env: &MediaEnv) -> Option<bool> {
        match self {
            Self::Feature(feature) => feature.eval_tri(env),
            Self::Not(inner) => kleene_not(inner.eval_tri(env)),
            Self::And(items) => {
                let mut acc = Some(true);
                for item in items {
                    acc = kleene_and(acc, item.eval_tri(env));
                    if acc == Some(false) {
                        return Some(false);
                    }
                }
                acc
            }
            Self::Or(items) => {
                let mut acc = Some(false);
                for item in items {
                    acc = kleene_or(acc, item.eval_tri(env));
                    if acc == Some(true) {
                        return Some(true);
                    }
                }
                acc
            }
            Self::Unknown => None,
        }
    }

    /// Returns `true` iff [`Self::eval_tri`] resolves to `Some(true)`.
    #[inline]
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.eval_tri(env) == Some(true)
    }
}

impl std::ops::Not for MediaCondition {
    type Output = Self;

    fn not(self) -> Self {
        Self::Not(Box::new(self))
    }
}

/// One `@media` query: optional leading `not`, a [`MediaType`], and an
/// optional [`MediaCondition`].
///
/// For condition-only queries (e.g. `@media (min-width: 600px)` or
/// `@media not (unknown: 1)`), set `media_type: MediaType::All`,
/// `negated: false`, and place any leading `not` inside `condition` so
/// `not (unknown)` evaluates via [`MediaCondition::Not`] to `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaQuery {
    /// Leading `not` modifier applied to the whole `<media-type> and <condition>`.
    pub negated: bool,
    pub media_type: MediaType,
    pub condition: Option<MediaCondition>,
}

impl Default for MediaQuery {
    fn default() -> Self {
        Self::all()
    }
}

impl MediaQuery {
    /// Unconditional `all` query.
    pub const fn all() -> Self {
        Self {
            negated: false,
            media_type: MediaType::All,
            condition: None,
        }
    }

    /// Condition-only query (implicitly `MediaType::All`, `negated: false`).
    pub const fn from_condition(condition: MediaCondition) -> Self {
        Self {
            negated: false,
            media_type: MediaType::All,
            condition: Some(condition),
        }
    }

    /// Media-type query with optional condition and optional leading `not`.
    pub const fn with_type(
        negated: bool,
        media_type: MediaType,
        condition: Option<MediaCondition>,
    ) -> Self {
        Self {
            negated,
            media_type,
            condition,
        }
    }

    /// True if this query matches every environment unconditionally (`all` with
    /// no condition and no negation).
    pub const fn is_unconditional(&self) -> bool {
        !self.negated && matches!(self.media_type, MediaType::All) && self.condition.is_none()
    }

    /// Evaluate under strict Kleene 3-valued logic.
    pub fn eval_tri(&self, env: &MediaEnv) -> Option<bool> {
        let type_val = Some(self.media_type.matches_env_type(env.media_type));
        let combined = match &self.condition {
            Some(cond) => kleene_and(type_val, cond.eval_tri(env)),
            None => type_val,
        };
        if self.negated {
            kleene_not(combined)
        } else {
            combined
        }
    }

    /// Returns `true` iff [`Self::eval_tri`] resolves to `Some(true)`.
    #[inline]
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.eval_tri(env) == Some(true)
    }
}

/// Comma-separated list of [`MediaQuery`] alternatives (disjunction).
///
/// An empty list matches unconditionally (same as `all`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaQueryList {
    pub queries: Vec<MediaQuery>,
}

impl MediaQueryList {
    /// A list containing one unconditional `all` query.
    pub fn all() -> Self {
        Self {
            queries: vec![MediaQuery::all()],
        }
    }

    /// Whether the list is empty or contains an unconditional `all` query.
    pub fn is_unconditional(&self) -> bool {
        self.queries.is_empty() || self.queries.iter().any(MediaQuery::is_unconditional)
    }

    /// Evaluate the list under Kleene 3-valued disjunction (`Or`).
    /// An empty list evaluates to `Some(true)`.
    pub fn eval_tri(&self, env: &MediaEnv) -> Option<bool> {
        if self.queries.is_empty() {
            return Some(true);
        }
        let mut acc = Some(false);
        for query in &self.queries {
            acc = kleene_or(acc, query.eval_tri(env));
            if acc == Some(true) {
                return Some(true);
            }
        }
        acc
    }

    /// Returns `true` iff [`Self::eval_tri`] resolves to `Some(true)`.
    #[inline]
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.eval_tri(env) == Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kleene_truth_tables_are_exact() {
        // NOT
        assert_eq!(kleene_not(Some(true)), Some(false));
        assert_eq!(kleene_not(Some(false)), Some(true));
        assert_eq!(kleene_not(None), None);

        // AND: false dominates unknown; true preserves unknown
        assert_eq!(kleene_and(Some(false), None), Some(false));
        assert_eq!(kleene_and(None, Some(false)), Some(false));
        assert_eq!(kleene_and(Some(true), None), None);
        assert_eq!(kleene_and(None, Some(true)), None);
        assert_eq!(kleene_and(None, None), None);
        assert_eq!(kleene_and(Some(true), Some(true)), Some(true));
        assert_eq!(kleene_and(Some(true), Some(false)), Some(false));
        assert_eq!(kleene_and(Some(false), Some(false)), Some(false));

        // OR: true dominates unknown; false preserves unknown
        assert_eq!(kleene_or(Some(true), None), Some(true));
        assert_eq!(kleene_or(None, Some(true)), Some(true));
        assert_eq!(kleene_or(Some(false), None), None);
        assert_eq!(kleene_or(None, Some(false)), None);
        assert_eq!(kleene_or(None, None), None);
        assert_eq!(kleene_or(Some(false), Some(false)), Some(false));
        assert_eq!(kleene_or(Some(true), Some(false)), Some(true));
        assert_eq!(kleene_or(Some(true), Some(true)), Some(true));
    }

    #[test]
    fn kleene_conditions_with_unknown_follow_mq4_spec() {
        let env = MediaEnv::desktop(800.0, 600.0, false);
        let t = MediaCondition::feature(MediaFeature::min_width(600.0));
        let f = MediaCondition::feature(MediaFeature::min_width(1200.0));
        let u = MediaCondition::Unknown;
        let uf = MediaCondition::feature(MediaFeature::Unknown);

        // Unknown => None, matches => false
        assert_eq!(u.eval_tri(&env), None);
        assert!(!u.matches(&env));
        assert_eq!(uf.eval_tri(&env), None);
        assert!(!uf.matches(&env));

        // Not(None) => None (does NOT become true!)
        let not_u = MediaCondition::negate(u.clone());
        assert_eq!(not_u.eval_tri(&env), None);
        assert!(!not_u.matches(&env));
        assert_eq!((!u.clone()).eval_tri(&env), None);

        // And(false, unknown) => Some(false)
        let f_and_u = MediaCondition::and(vec![f.clone(), u.clone()]);
        assert_eq!(f_and_u.eval_tri(&env), Some(false));
        assert!(!f_and_u.matches(&env));

        // Not(And(false, unknown)) => Some(true)!
        let not_f_and_u = MediaCondition::negate(f_and_u);
        assert_eq!(not_f_and_u.eval_tri(&env), Some(true));
        assert!(not_f_and_u.matches(&env));

        // And(true, unknown) => None
        let t_and_u = MediaCondition::and(vec![t.clone(), u.clone()]);
        assert_eq!(t_and_u.eval_tri(&env), None);
        assert!(!t_and_u.matches(&env));

        // Or(true, unknown) => Some(true)
        let t_or_u = MediaCondition::or(vec![t, u.clone()]);
        assert_eq!(t_or_u.eval_tri(&env), Some(true));
        assert!(t_or_u.matches(&env));

        // Or(false, unknown) => None
        let f_or_u = MediaCondition::or(vec![f, u]);
        assert_eq!(f_or_u.eval_tri(&env), None);
        assert!(!f_or_u.matches(&env));
    }

    #[test]
    fn media_query_negation_and_media_types() {
        let screen_env = MediaEnv::desktop(1024.0, 768.0, false);

        // `not print` matches on screen
        let not_print = MediaQuery::with_type(true, MediaType::Print, None);
        assert_eq!(not_print.eval_tri(&screen_env), Some(true));
        assert!(not_print.matches(&screen_env));

        // `not other` matches on screen (unknown media type is false, not unknown)
        let not_other = MediaQuery::with_type(true, MediaType::Other, None);
        assert_eq!(not_other.eval_tri(&screen_env), Some(true));
        assert!(not_other.matches(&screen_env));

        // `not print and (unknown)` => Not(And(false, None)) => Not(false) => Some(true)
        let not_print_and_unknown =
            MediaQuery::with_type(true, MediaType::Print, Some(MediaCondition::Unknown));
        assert_eq!(not_print_and_unknown.eval_tri(&screen_env), Some(true));

        // `not screen and (unknown)` => Not(And(true, None)) => Not(None) => None
        let not_screen_and_unknown =
            MediaQuery::with_type(true, MediaType::Screen, Some(MediaCondition::Unknown));
        assert_eq!(not_screen_and_unknown.eval_tri(&screen_env), None);
        assert!(!not_screen_and_unknown.matches(&screen_env));

        // `not screen and (min-width: 1400px)` at 1024px => Not(And(true, false)) => Some(true)
        let not_wide_screen = MediaQuery::with_type(
            true,
            MediaType::Screen,
            Some(MediaCondition::feature(MediaFeature::min_width(1400.0))),
        );
        assert_eq!(not_wide_screen.eval_tri(&screen_env), Some(true));
        assert!(not_wide_screen.matches(&screen_env));

        // Print environment matches `print` and rejects `screen`
        let print_env = MediaEnv {
            media_type: MediaType::Print,
            ..MediaEnv::default()
        };
        assert!(MediaQuery::with_type(false, MediaType::Print, None).matches(&print_env));
        assert!(!MediaQuery::with_type(false, MediaType::Screen, None).matches(&print_env));
        assert!(MediaQuery::all().matches(&print_env));
    }

    #[test]
    fn all_supported_media_features_evaluate_accurately() {
        let env = MediaEnv {
            media_type: MediaType::Screen,
            width: 1600.0,
            height: 900.0,
            color_scheme: MediaColorScheme::Dark,
            reduced_motion: MediaReducedMotion::Reduce,
            contrast: MediaContrast::More,
            hover: MediaHover::Hover,
            pointer: MediaPointer::Fine,
            resolution_dppx: 2.0,
        };

        // Width & height ranges
        assert!(MediaFeature::width(1600.0).matches(&env));
        assert!(MediaFeature::min_width(1600.0).matches(&env));
        assert!(MediaFeature::max_width(1600.0).matches(&env));
        assert!(MediaFeature::Width(MediaComparison::Gt, 1599.0).matches(&env));
        assert!(!MediaFeature::Width(MediaComparison::Gt, 1600.0).matches(&env));
        assert!(MediaFeature::Width(MediaComparison::Lt, 1601.0).matches(&env));
        assert!(MediaFeature::height(900.0).matches(&env));
        assert!(MediaFeature::min_height(900.0).matches(&env));
        assert!(MediaFeature::max_height(900.0).matches(&env));

        // Aspect ratio: 1600x900 == 16/9 exact via cross-multiplication
        assert!(MediaFeature::aspect_ratio(16.0, 9.0).matches(&env));
        assert!(MediaFeature::min_aspect_ratio(16.0, 9.0).matches(&env));
        assert!(MediaFeature::max_aspect_ratio(16.0, 9.0).matches(&env));
        assert!(MediaFeature::min_aspect_ratio(4.0, 3.0).matches(&env));
        assert!(!MediaFeature::max_aspect_ratio(4.0, 3.0).matches(&env));
        assert!(!MediaFeature::aspect_ratio(16.0, 0.0).matches(&env));

        // Orientation (CSS MQ4: height >= width is Portrait; width > height is Landscape)
        assert!(MediaFeature::Orientation(MediaOrientation::Landscape).matches(&env));
        assert!(!MediaFeature::Orientation(MediaOrientation::Portrait).matches(&env));
        let square = MediaEnv::desktop(500.0, 500.0, false);
        assert!(MediaFeature::Orientation(MediaOrientation::Portrait).matches(&square));
        assert!(!MediaFeature::Orientation(MediaOrientation::Landscape).matches(&square));

        // Preferences & interaction
        assert!(MediaFeature::PrefersColorScheme(MediaColorScheme::Dark).matches(&env));
        assert!(!MediaFeature::PrefersColorScheme(MediaColorScheme::Light).matches(&env));
        assert!(MediaFeature::PrefersReducedMotion(MediaReducedMotion::Reduce).matches(&env));
        assert!(
            !MediaFeature::PrefersReducedMotion(MediaReducedMotion::NoPreference).matches(&env)
        );
        assert!(MediaFeature::PrefersContrast(MediaContrast::More).matches(&env));
        assert!(!MediaFeature::PrefersContrast(MediaContrast::Less).matches(&env));
        assert!(!MediaFeature::PrefersContrast(MediaContrast::Custom).matches(&env));
        assert!(MediaFeature::Hover(MediaHover::Hover).matches(&env));
        assert!(!MediaFeature::Hover(MediaHover::None).matches(&env));
        assert!(MediaFeature::Pointer(MediaPointer::Fine).matches(&env));
        assert!(!MediaFeature::Pointer(MediaPointer::Coarse).matches(&env));
        assert!(!MediaFeature::Pointer(MediaPointer::None).matches(&env));

        // Resolution
        assert!(MediaFeature::resolution(2.0).matches(&env));
        assert!(MediaFeature::min_resolution(1.5).matches(&env));
        assert!(MediaFeature::max_resolution(2.0).matches(&env));
        assert!(!MediaFeature::min_resolution(3.0).matches(&env));
    }

    #[test]
    fn media_query_list_disjunction_and_unconditional() {
        let env = MediaEnv::desktop(800.0, 600.0, false);
        assert!(MediaQueryList::default().is_unconditional());
        assert_eq!(MediaQueryList::default().eval_tri(&env), Some(true));
        assert!(MediaQueryList::all().is_unconditional());

        // [unknown, true] => Some(true)
        let list = MediaQueryList {
            queries: vec![
                MediaQuery::from_condition(MediaCondition::Unknown),
                MediaQuery::from_condition(MediaCondition::feature(MediaFeature::max_width(900.0))),
            ],
        };
        assert!(!list.is_unconditional());
        assert_eq!(list.eval_tri(&env), Some(true));
        assert!(list.matches(&env));

        // [unknown, false] => None -> matches == false
        let list_unknown = MediaQueryList {
            queries: vec![
                MediaQuery::from_condition(MediaCondition::Unknown),
                MediaQuery::from_condition(MediaCondition::feature(MediaFeature::min_width(
                    1200.0,
                ))),
            ],
        };
        assert_eq!(list_unknown.eval_tri(&env), None);
        assert!(!list_unknown.matches(&env));
    }
}
