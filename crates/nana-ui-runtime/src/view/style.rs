//! Styles compiled from CSS at build time.
//!
//! The `.vue` compiler (and `css!`) parse and match a view's stylesheet
//! while the application builds, with the CSS engine of `nana-ui-css`. What
//! reaches the running program is data: for each element, the rules that
//! apply to it in cascade order, each as a [`StylePatch`] (the Style Model
//! fields its declarations set) and the conditional classes it needs. No
//! CSS is parsed and no selector is matched at run time, and this crate has
//! no CSS in it.
//!
//! A [`Sheet`] is a stylesheet's class rules in cascade order. An element
//! names its classes with [`El::class`] and [`El::class_when`]; the sheet
//! picks the rules those classes can match, once per distinct set of
//! classes, into a [`StyleSite`]. The site composes the element's base
//! layout with the patches whose classes are active, once per distinct base
//! and active set, and hands out the same shared layout to every instance
//! after that.
//!
//! A rule inside a CSS `@container` block carries its query as data
//! ([`SheetQuery`]): the container it reads, the axis, and the extents where
//! it holds. Composing leaves those rules out. Under the same active set,
//! the site cuts the container's extent at every bound their queries use and
//! composes each bucket with the rules that hold there, in cascade order, a
//! later plain rule still winning; what a bucket writes over the authored
//! composition is its [`StyleVariant`]. The element follows the result as a
//! [`ResponsiveRule`] on the nearest query container, which the runtime
//! measures; nothing here does.

use nana_ui_core::sort::CompactSort as _;
use std::collections::HashMap;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use nana_ui_core::{
    FontFaceRule, HotSheetAxis, HotSheetQuery, HotStyleRule, HotStyleSheet, LayoutStyle, MediaEnv,
    MediaQueryList,
};

use super::controls::StyledComponent;
use super::node::{DynBinding, DynProp, El, NodeBindings};
use super::prop::{FieldWrite, IntoProp, PropSource};
use super::reactive::{Signal, signal};
use super::transition::{Implicit, KeyframeAnimation};
use crate::{
    ComponentView, MAX_RESPONSIVE_BREAKPOINTS, MutationQueue, ResponsiveAxis, ResponsiveContainer,
    ResponsiveRule, StableNodeId, StyleVariant,
};

pub use nana_ui_core::StylePatch;

#[doc(hidden)]
pub const PSEUDO_BIT_SHIFT: u32 = 59;
#[doc(hidden)]
pub const MAX_CONDITIONAL_CLASSES: usize = 59;
#[doc(hidden)]
pub const PSEUDO_HOVER: u8 = 1 << 0;
#[doc(hidden)]
pub const PSEUDO_FOCUS: u8 = 1 << 1;
#[doc(hidden)]
pub const PSEUDO_FOCUS_VISIBLE: u8 = 1 << 2;
#[doc(hidden)]
pub const PSEUDO_ACTIVE: u8 = 1 << 3;
#[doc(hidden)]
pub const PSEUDO_SELECTED: u8 = 1 << 4;

/// Codegen helpers emitted by compiled stylesheet patches; not part of the
/// supported public API.
#[doc(hidden)]
pub mod __patch {
    pub use nana_ui_core::box_layout::intern_calc;
    pub use nana_ui_core::dynamic_layout::{
        AdaptationProfile, AxisElasticity, DiscreteAdaptation, DiscreteCandidate, ElasticFloor,
        ElasticLength, KeepWith, LayoutCost,
    };
    pub use nana_ui_core::shared::Shared;
    pub use nana_ui_core::style_patch::set;
    pub use nana_ui_core::{
        AlignSpec, AnchorSpec, AnimatableProperty, AnimationDirection, AnimationFillMode,
        AnimationIteration, AnimationPlayState, BackdropFilter, BackfaceVisibility,
        BackgroundImage, BackgroundImageFit, BackgroundPosition, BackgroundRepeat,
        BorderImageSlice, BorderImageSpec, BorderImageTile, BorderSides, BorderStyle, BorderWidth,
        BoxShadowSpec, BoxSizing, CalcBinOp, CalcExpr, CalcExprRef, ClipCircle, ClipEllipse,
        ClipInset, ClipPath, ClipPoint, ClipShapeRadius, ColorFilter, ContainerStyle,
        ContainerType, ControlHeight, ControlPadding, ControlSize, CornerShape, CssGradient,
        CursorSpec, CustomCursorError, CustomCursorSpec, CustomCursorVariant, CustomCursorVector,
        DirSpec, DisplaySpec, DominantBaselineSpec, Easing, EasingRole, ElevationRole,
        FilterDropShadow, FilterValue, FlexDirection, FlexWrap, FontFaceRule, FontFaceSource,
        FontFeatureSetting, FontKerningSpec, FontStyleSpec, FontVariationSetting, GradientStop,
        GridAutoFlow, GridLine, GridPlacement, GridRepeatAuto, GridStyle, GridTemplateAreas,
        GridTrack, GridTrackListUnsupported, HitTestBehavior, HyphensSpec, Icon, IconData,
        IconGeometry, IconPathCommand, IconShape, ImageSampling, JustifySpec, LengthAtom,
        LengthSpec, LineBreakSpec, LineHeightSpec, LineRole, LinearGradient, LogicalBoxEdges,
        LogicalEdge, LogicalEdges, MaskImage, MaskKind, MaterialProgram, MaterialRef,
        MaterialTexture, MediaColorScheme, MediaComparison, MediaCondition, MediaContrast,
        MediaEnv, MediaFeature, MediaHover, MediaOrientation, MediaPointer, MediaQuery,
        MediaQueryList, MediaReducedMotion, MediaType, MixBlendMode, MotionCurve, MotionRole,
        OutlineSpec, OutlineStyle, OverflowSpec, OverflowWrapSpec, PaddingSpec, PaintColor,
        PaintColorSlots, PaintMat4, PaintStyle, PaintTransform, PointerEventsSpec,
        PopoverAlignment, PopoverPlacement, PositionSpec, RadialGradient, RadiusTier,
        ScrollEdgeFade, ScrollbarSkin, SemanticColor, SemanticColorMix, SemanticColorRole,
        ShapeRole, SpacingRole, SpacingStep, SquareSize, StepJump, StyleOp, SurfacePadding,
        SurfaceRole, SystemCursor, TextAlignSpec, TextAutospaceSpec, TextDecorationLine,
        TextDecorationStyle, TextEmphasis, TextEmphasisColor, TextEmphasisPosition,
        TextEmphasisShape, TextEmphasisStyle, TextJustifySpec, TextOrientationSpec, TextShadowSpec,
        TextSpacingTrimSpec, TextStrokeJoin, TextStrokePlacement, TextStrokeSpec, TextStyleExt,
        TextTypography, TextWeight, TextWrapBreak, TextWrapStyleSpec, ThemeColorPairRef, ThemeExpr,
        ThemeProperty, ThemeToken, TransformBox, TransformOrigin, TransformStyle, TypeRole,
        UserSelectSpec, VerticalAlignSpec, ViewportAxis, VisibilitySpec, WhiteSpaceSpec,
        WindowResizeEdge, WordBreakSpec, WritingModeSpec, intern_theme_color_pair,
    };
}

/// Compiled `@media` query guard on a [`Sheet`] rule or [`StyleSite`],
/// constructed lazily once from a static builder function and cached for
/// the process lifetime.
#[doc(hidden)]
pub struct SheetMediaQuery {
    build: fn() -> MediaQueryList,
    cached: OnceLock<MediaQueryList>,
}

impl SheetMediaQuery {
    #[doc(hidden)]
    pub const fn new(build: fn() -> MediaQueryList) -> Self {
        Self {
            build,
            cached: OnceLock::new(),
        }
    }

    /// The cached [`MediaQueryList`].
    pub fn query_list(&self) -> &MediaQueryList {
        self.cached.get_or_init(self.build)
    }

    /// Whether this `@media` query matches `env` under Kleene 3-valued logic.
    #[inline]
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.query_list().matches(env)
    }
}

/// Reactive access to the active `@media` evaluation environment (viewport
/// dimensions, scale factor, light/dark color scheme, reduced motion, contrast,
/// pointer/hover capabilities). Initialized on mount and updated whenever the
/// world's media environment changes.
pub fn media_env_signal() -> Signal<MediaEnv> {
    let value = signal(MediaEnv::default());
    super::reactive::on_mount(move |cx| {
        value.set_if_changed(cx.media_env());
        cx.media_env_signals.push(value);
    });
    value
}

/// Reactive interactive pseudo-state handle (`:hover` / `:focus` /
/// `:focus-visible` / `:active` / `:selected`) exposed by an ancestor element
/// via [`El::track_interactive_state`] so compiled descendant rules
/// (`.card:hover .icon`, `.item:active .label`) react without a runtime DOM
/// selector engine.
#[derive(Clone, Copy)]
pub struct InteractiveStateHandle {
    signal: Signal<u8>,
}

impl Default for InteractiveStateHandle {
    #[track_caller]
    fn default() -> Self {
        Self::new()
    }
}

impl InteractiveStateHandle {
    /// Create a new interactive-state handle in the current reactive scope.
    #[track_caller]
    pub fn new() -> Self {
        Self {
            signal: signal(0u8),
        }
    }

    /// Current active pseudo-state bitmask (tracked).
    #[inline]
    pub fn bits(self) -> u8 {
        self.signal.get()
    }

    /// Whether every pseudo bit in `mask` is currently active (tracked).
    #[inline]
    pub fn matches(self, mask: u8) -> bool {
        (self.signal.get() & mask) == mask
    }

    /// Whether the tracked element (or a descendant) is hovered (tracked).
    #[inline]
    pub fn hovered(self) -> bool {
        self.matches(PSEUDO_HOVER)
    }

    /// Whether the tracked element (or a descendant) is pressed/active (tracked).
    #[inline]
    pub fn active(self) -> bool {
        self.matches(PSEUDO_ACTIVE)
    }

    /// Whether the tracked element holds focus (tracked).
    #[inline]
    pub fn focused(self) -> bool {
        self.matches(PSEUDO_FOCUS)
    }

    /// Whether the tracked element holds keyboard-visible focus (tracked).
    #[inline]
    pub fn focus_visible(self) -> bool {
        self.matches(PSEUDO_FOCUS_VISIBLE)
    }

    /// Whether the tracked element is selected (tracked).
    #[inline]
    pub fn selected(self) -> bool {
        self.matches(PSEUDO_SELECTED)
    }

    pub(crate) fn signal(self) -> Signal<u8> {
        self.signal
    }
}

/// Create a new [`InteractiveStateHandle`] to attach to an ancestor element
/// via [`El::track_interactive_state`] and consume from descendants via
/// [`El::ancestor_pseudo`] or [`El::class_when`].
#[track_caller]
pub fn interactive_state() -> InteractiveStateHandle {
    InteractiveStateHandle::new()
}

/// The CSS `@container` query of a rule, compiled at build time: the
/// container it reads (the nearest one named `name`, or the nearest query
/// container eligible for `axis`) and the extents of `axis` where it holds,
/// half-open `[lo, hi)`, ascending and disjoint. `lo` may be
/// `f32::NEG_INFINITY` and `hi` `f32::INFINITY`; an inclusive bound such as
/// `max-width: 480px` is already `480f32.next_up()`.
#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub struct SheetQuery {
    name: Option<&'static str>,
    axis: ResponsiveAxis,
    intervals: &'static [(f32, f32)],
}

impl SheetQuery {
    #[doc(hidden)]
    pub const fn new(
        name: Option<&'static str>,
        axis: ResponsiveAxis,
        intervals: &'static [(f32, f32)],
    ) -> Self {
        Self {
            name,
            axis,
            intervals,
        }
    }

    /// Whether the query holds for a container measuring `extent` on its
    /// axis.
    fn holds_at(&self, extent: f32) -> bool {
        self.intervals
            .iter()
            .any(|&(lo, hi)| lo <= extent && extent < hi)
    }
}

/// One compiled rule of a site: the conditional classes it needs (bit `i`
/// is the element's `i`-th conditional class), its patch, and the
/// `@container` query that guards it.
type SiteRule = (u64, &'static StylePatch, Option<&'static SheetQuery>);
type SiteTransition = (u64, &'static [Implicit]);
type SiteAnimation = (u64, &'static [KeyframeAnimation]);

/// Dev-only metadata connecting a compiled `Sheet` to its view's hot `<style>`
/// patch table.
#[derive(Clone, Copy)]
struct HotSheetMeta {
    view: &'static str,
    classes: &'static [&'static str],
}

/// Dev-only metadata for one resolved `StyleSite` in hot mode.
#[derive(Clone, Copy)]
struct HotSiteMeta {
    view: &'static str,
    fixed_names: &'static [&'static str],
    conditional_names: &'static [&'static str],
}

/// One element's compiled rules, in cascade order: each patch applies
/// while every conditional class, interactive pseudo, and `@media` query in
/// its mask is active, and a guarded one only in the buckets of its container
/// where its query holds.
pub struct StyleSite {
    rules: &'static [SiteRule],
    /// Optional per-rule `@media` guard (parallel to `rules`); empty when no
    /// rule in this site is guarded by `@media`.
    rule_media: &'static [Option<&'static SheetMediaQuery>],
    transitions: &'static [SiteTransition],
    animations: &'static [SiteAnimation],
    used_pseudos: u8,
    hot: Option<HotSiteMeta>,
    /// What every instance of the site shares.
    composed: Mutex<Option<Composed>>,
}

/// How many of the bases a site saw last it keeps alive itself.
const RECENT_BASES: usize = 4;

/// A site's bases, inline layers, compositions and responsive rules. Held
/// weakly by base (and inline layer): entries no instance keeps any more, and
/// that are not among the last [`RECENT_BASES`] the site saw, take what was
/// made from them with them at the next prune.
#[derive(Default)]
struct Composed {
    /// The distinct bases seen, one per value.
    bases: Vec<Weak<LayoutStyle>>,
    /// The last few distinct bases, held: an element without conditional
    /// classes swaps its base for the composition, so nothing else keeps
    /// the base, and the next equal element would compose again.
    recent: Vec<Arc<LayoutStyle>>,
    /// The distinct Layer 2 inline stacks seen, one per value.
    inlines: Vec<Weak<InlineLayer>>,
    /// The last few distinct inline stacks, held (mirrors [`Self::recent`]).
    recent_inlines: Vec<Arc<InlineLayer>>,
    /// `(base pointer, active classes, active media mask) -> composed`.
    by_base: HashMap<(usize, u64, u64), (Weak<LayoutStyle>, Arc<LayoutStyle>)>,
    /// `(base pointer, active classes, active media mask, inline pointer) -> composed`.
    by_inline:
        HashMap<(usize, u64, u64, usize), (Weak<LayoutStyle>, Weak<InlineLayer>, Arc<LayoutStyle>)>,
    /// `(base pointer, active classes, active media mask) -> the rule its guarded rules give`.
    responsive: HashMap<(usize, u64, u64), (Weak<LayoutStyle>, Option<Arc<ResponsiveRule>>)>,
    /// Entries at the last prune; the next runs when that doubles.
    pruned_at: usize,
    /// Compositions made, for tests.
    #[cfg(test)]
    compositions: usize,
}

impl Composed {
    /// The live base equal to `base`, `base` itself when none is, kept among
    /// the recent ones. Pointers first: a value comparison of a whole layout
    /// runs only for a base pointer not seen before.
    fn canonical(&mut self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        let known = |weak: &Weak<LayoutStyle>| weak.upgrade();
        let found = self
            .bases
            .iter()
            .filter_map(known)
            .find(|known| Arc::ptr_eq(known, base))
            .or_else(|| {
                self.bases
                    .iter()
                    .filter_map(known)
                    .find(|known| **known == **base)
            })
            .unwrap_or_else(|| {
                self.bases.push(Arc::downgrade(base));
                Arc::clone(base)
            });
        if !self.recent.iter().any(|kept| Arc::ptr_eq(kept, &found)) {
            self.recent.truncate(RECENT_BASES - 1);
            self.recent.insert(0, Arc::clone(&found));
        }
        found
    }

    /// The live Layer 2 inline stack equal to `inline`, sharing one `Arc`
    /// across all instances with identical inline overrides.
    fn canonical_inline(&mut self, inline: InlineLayer) -> Arc<InlineLayer> {
        let found = self
            .inlines
            .iter()
            .filter_map(Weak::upgrade)
            .find(|known| **known == inline)
            .unwrap_or_else(|| {
                let arc = Arc::new(inline);
                self.inlines.push(Arc::downgrade(&arc));
                arc
            });
        self.keep_recent_inline(&found);
        found
    }

    fn canonical_inline_ref(&mut self, inline: &InlineLayer) -> Arc<InlineLayer> {
        let found = self
            .inlines
            .iter()
            .filter_map(Weak::upgrade)
            .find(|known| **known == *inline)
            .unwrap_or_else(|| {
                let arc = Arc::new(inline.clone());
                self.inlines.push(Arc::downgrade(&arc));
                arc
            });
        self.keep_recent_inline(&found);
        found
    }

    fn canonical_inline_arc(&mut self, inline: &Arc<InlineLayer>) -> Arc<InlineLayer> {
        let found = self
            .inlines
            .iter()
            .filter_map(Weak::upgrade)
            .find(|known| Arc::ptr_eq(known, inline))
            .or_else(|| {
                self.inlines
                    .iter()
                    .filter_map(Weak::upgrade)
                    .find(|known| **known == **inline)
            })
            .unwrap_or_else(|| {
                self.inlines.push(Arc::downgrade(inline));
                Arc::clone(inline)
            });
        self.keep_recent_inline(&found);
        found
    }

    fn keep_recent_inline(&mut self, found: &Arc<InlineLayer>) {
        if !self
            .recent_inlines
            .iter()
            .any(|kept| Arc::ptr_eq(kept, found))
        {
            self.recent_inlines.truncate(RECENT_BASES - 1);
            self.recent_inlines.insert(0, Arc::clone(found));
        }
    }

    /// Drop what belongs to bases or inline layers nothing holds, once the
    /// maps have doubled.
    fn prune(&mut self) {
        let entries = self.by_base.len() + self.by_inline.len() + self.responsive.len();
        if entries < (self.pruned_at * 2).max(64) {
            return;
        }
        self.bases.retain(|base| base.strong_count() > 0);
        self.inlines.retain(|inline| inline.strong_count() > 0);
        self.by_base.retain(|_, (base, _)| base.strong_count() > 0);
        self.by_inline
            .retain(|_, (base, inline, _)| base.strong_count() > 0 && inline.strong_count() > 0);
        self.responsive
            .retain(|_, (base, _)| base.strong_count() > 0);
        self.pruned_at = self.by_base.len() + self.by_inline.len() + self.responsive.len();
    }
}

/// The container `queries` read and the finite bounds they use, ascending
/// and unique: every bound is a breakpoint, so each query holds across a
/// whole bucket or nowhere in it. `None` without a query; also `None`, and a
/// diagnostic naming where the element's classes were given (`at`), when
/// they ask more than one container or axis or use more than
/// [`MAX_RESPONSIVE_BREAKPOINTS`] bounds.
fn plan(
    queries: &[&SheetQuery],
    at: &'static Location<'static>,
) -> Option<(Option<&'static str>, ResponsiveAxis, Vec<f32>)> {
    let first = queries.first()?;
    let mut breakpoints: Vec<f32> = queries
        .iter()
        .flat_map(|query| query.intervals.iter())
        .flat_map(|&(lo, hi)| [lo, hi])
        .filter(|bound| bound.is_finite())
        .collect();
    breakpoints.compact_sort_by(f32::total_cmp);
    breakpoints.dedup();
    let why = if queries
        .iter()
        .any(|query| query.name != first.name || query.axis != first.axis)
    {
        "ask more than one container or axis"
    } else if breakpoints.len() > MAX_RESPONSIVE_BREAKPOINTS {
        "use more breakpoints than one responsive rule holds"
    } else {
        return Some((first.name, first.axis, breakpoints));
    };
    nana_diagnostics::fault!(
        nana_diagnostics::framework::runtime::VIEW_CONTAINER_QUERY_UNSUPPORTED,
        breakpoints = breakpoints.len() as u64;
        "the @container rules of the element whose classes are given at {at} {why}: none of \
         them apply"
    );
    None
}

impl StyleSite {
    #[doc(hidden)]
    pub const fn new(rules: &'static [SiteRule]) -> Self {
        Self {
            rules,
            rule_media: &[],
            transitions: &[],
            animations: &[],
            used_pseudos: 0,
            hot: None,
            composed: Mutex::new(None),
        }
    }

    #[doc(hidden)]
    pub const fn with_motion(
        rules: &'static [SiteRule],
        transitions: &'static [SiteTransition],
        animations: &'static [SiteAnimation],
        used_pseudos: u8,
    ) -> Self {
        Self {
            rules,
            rule_media: &[],
            transitions,
            animations,
            used_pseudos,
            hot: None,
            composed: Mutex::new(None),
        }
    }

    /// Attach per-rule `@media` guards (parallel to `rules`) to this site.
    #[doc(hidden)]
    pub const fn with_media(
        mut self,
        rule_media: &'static [Option<&'static SheetMediaQuery>],
    ) -> Self {
        self.rule_media = rule_media;
        self
    }

    const fn with_hot(
        rules: &'static [SiteRule],
        rule_media: &'static [Option<&'static SheetMediaQuery>],
        transitions: &'static [SiteTransition],
        animations: &'static [SiteAnimation],
        used_pseudos: u8,
        hot: Option<HotSiteMeta>,
    ) -> Self {
        Self {
            rules,
            rule_media,
            transitions,
            animations,
            used_pseudos,
            hot,
            composed: Mutex::new(None),
        }
    }

    pub(crate) fn has_media(&self) -> bool {
        self.rule_media.iter().any(Option::is_some)
    }

    fn eval_media_mask(&self, env: Option<&MediaEnv>) -> Option<u64> {
        if !self.has_media() {
            return Some(0);
        }
        let default_env = MediaEnv::default();
        let env = env.unwrap_or(&default_env);
        let mut mask = 0u64;
        for (bit, mq) in self.rule_media.iter().flatten().enumerate() {
            if bit >= 64 {
                return None;
            }
            if mq.matches(env) {
                mask |= 1u64 << bit;
            }
        }
        Some(mask)
    }

    fn rule_media_matches(&self, idx: usize, env: Option<&MediaEnv>) -> bool {
        match self.rule_media.get(idx).copied().flatten() {
            None => true,
            Some(mq) => match env {
                Some(env) => mq.matches(env),
                None => mq.matches(&MediaEnv::default()),
            },
        }
    }

    pub(crate) const fn used_pseudos(&self) -> u8 {
        self.used_pseudos
    }

    pub(crate) const fn has_dynamic_motion(&self) -> bool {
        !self.animations.is_empty() || !self.transitions.is_empty()
    }

    pub(crate) fn transitions_for(&self, active: u64) -> Vec<Implicit> {
        let mut out: Vec<Implicit> = Vec::new();
        for &(needs, items) in self.transitions {
            if needs & !active == 0 {
                for &item in items {
                    if let Some(existing) = out.iter_mut().find(|cur| cur.property == item.property)
                    {
                        *existing = item;
                    } else {
                        out.push(item);
                    }
                }
            }
        }
        out
    }

    pub(crate) fn animations_for(&self, active: u64) -> Vec<KeyframeAnimation> {
        let mut out: Vec<KeyframeAnimation> = Vec::new();
        for &(needs, items) in self.animations {
            if needs & !active == 0 {
                for &item in items {
                    if item.name.is_empty() {
                        for cur in &mut out {
                            cur.play_state = item.play_state;
                        }
                        continue;
                    }
                    if item.name.eq_ignore_ascii_case("none") {
                        out.clear();
                        continue;
                    }
                    if let Some(existing) = out
                        .iter_mut()
                        .find(|cur| cur.name == item.name && cur.property == item.property)
                    {
                        *existing = item;
                    } else {
                        out.push(item);
                    }
                }
            }
        }
        out
    }

    /// Whether any rule of the site is guarded by an `@container` query.
    fn has_queries(&self) -> bool {
        self.rules.iter().any(|(_, _, query)| query.is_some())
    }

    /// The base layout equal to `base` that this site has composed from
    /// before, or `base` itself. A binding that keeps its base holds this
    /// one: every instance then shares it, and composing finds it by
    /// pointer instead of comparing whole layouts.
    fn shared_base(&self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        let mut composed = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        composed
            .get_or_insert_with(Composed::default)
            .canonical(base)
    }

    /// Canonicalize a Layer 2 [`InlineLayer`] for this site so repeated
    /// instances with identical inline overrides share one `Arc<InlineLayer>`
    /// and hit the site's `(base, active, media, inline)` composition cache.
    fn shared_inline(&self, inline: InlineLayer) -> Arc<InlineLayer> {
        let mut composed = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        composed
            .get_or_insert_with(Composed::default)
            .canonical_inline(inline)
    }

    /// `base` with the patches that apply under `active`, in order. A rule
    /// guarded by an `@container` query is left out: it applies through
    /// [`Self::responsive`].
    pub fn compose(&self, base: &Arc<LayoutStyle>, active: u64) -> Arc<LayoutStyle> {
        self.compose_with_media(base, active, None)
    }

    /// Like [`Self::compose`], evaluating `@media`-guarded rules against `env`.
    pub fn compose_for_media(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        env: &MediaEnv,
    ) -> Arc<LayoutStyle> {
        self.compose_with_media(base, active, Some(env))
    }

    fn compose_with_media(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let layout = self.compose_in(composed, &base, active, env);
        composed.prune();
        layout
    }

    /// [`Self::compose`] over a canonical `base`, with the site's state held.
    fn compose_in(
        &self,
        composed: &mut Composed,
        base: &Arc<LayoutStyle>,
        active: u64,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let media_mask = self.eval_media_mask(env);
        if let Some(media_mask) = media_mask {
            let key = (Arc::as_ptr(base) as usize, active, media_mask);
            if let Some((known, layout)) = composed.by_base.get(&key)
                && known
                    .upgrade()
                    .is_some_and(|known| Arc::ptr_eq(&known, base))
            {
                return Arc::clone(layout);
            }
            let applying = self
                .rules
                .iter()
                .enumerate()
                .filter(|(idx, (needs, _, query))| {
                    query.is_none() && needs & !active == 0 && self.rule_media_matches(*idx, env)
                })
                .map(|(_, (_, patch, _))| *patch);
            let layout = Arc::new(apply(base, applying));
            #[cfg(test)]
            {
                composed.compositions += 1;
            }
            composed
                .by_base
                .insert(key, (Arc::downgrade(base), Arc::clone(&layout)));
            return layout;
        }
        // Fallback for sites with > 64 distinct @media rules: compose directly without mask cache.
        let applying = self
            .rules
            .iter()
            .enumerate()
            .filter(|(idx, (needs, _, query))| {
                query.is_none() && needs & !active == 0 && self.rule_media_matches(*idx, env)
            })
            .map(|(_, (_, patch, _))| *patch);
        #[cfg(test)]
        {
            composed.compositions += 1;
        }
        Arc::new(apply(base, applying))
    }

    /// The rule the site's `@container` rules give an element over `base`
    /// under `active`: one bucket per stretch of the container's extent
    /// between the bounds their queries use, each the composition with the
    /// guarded rules that hold there written over the authored one, in
    /// cascade order. `None` when no guarded rule applies under `active`,
    /// when none changes anything, or when they cannot make one rule: they
    /// ask more than one container or axis, or use more than
    /// [`MAX_RESPONSIVE_BREAKPOINTS`] bounds. The last is a diagnostic,
    /// naming where the element's classes were given (`at`). One rule per
    /// base and active set: an equal call returns the same one.
    #[cfg(test)]
    pub(crate) fn responsive(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        at: &'static Location<'static>,
    ) -> Option<Arc<ResponsiveRule>> {
        self.responsive_with_media(base, active, None, at)
    }

    fn responsive_with_media(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        env: Option<&MediaEnv>,
        at: &'static Location<'static>,
    ) -> Option<Arc<ResponsiveRule>> {
        if !self.has_queries() {
            return None;
        }
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let media_mask = self.eval_media_mask(env);
        if let Some(media_mask) = media_mask {
            let key = (Arc::as_ptr(&base) as usize, active, media_mask);
            if let Some((known, rule)) = composed.responsive.get(&key)
                && known
                    .upgrade()
                    .is_some_and(|known| Arc::ptr_eq(&known, &base))
            {
                return rule.clone();
            }
            let rule = self.plan_in(composed, &base, active, env, at).map(Arc::new);
            composed
                .responsive
                .insert(key, (Arc::downgrade(&base), rule.clone()));
            composed.prune();
            return rule;
        }
        self.plan_in(composed, &base, active, env, at).map(Arc::new)
    }

    /// [`Self::responsive`], uncached.
    fn plan_in(
        &self,
        composed: &mut Composed,
        base: &Arc<LayoutStyle>,
        active: u64,
        env: Option<&MediaEnv>,
        at: &'static Location<'static>,
    ) -> Option<ResponsiveRule> {
        let applying: Vec<&SiteRule> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(idx, (needs, _, _))| {
                needs & !active == 0 && self.rule_media_matches(*idx, env)
            })
            .map(|(_, rule)| rule)
            .collect();
        let queries: Vec<&SheetQuery> = applying.iter().filter_map(|rule| rule.2).collect();
        let (name, axis, breakpoints) = plan(&queries, at)?;
        let authored = self.compose_in(composed, base, active, env);
        let variants: Vec<Option<StyleVariant>> = (0..=breakpoints.len())
            .map(|bucket| {
                // The bucket's lower bound decides: no query changes inside
                // a bucket.
                let lower = bucket
                    .checked_sub(1)
                    .map_or(f32::NEG_INFINITY, |below| breakpoints[below]);
                let styled = apply(
                    base,
                    applying
                        .iter()
                        .filter(|(_, _, query)| query.is_none_or(|query| query.holds_at(lower)))
                        .map(|(_, patch, _)| *patch),
                );
                StyleVariant::between(&authored, &styled)
            })
            .collect();
        if variants.iter().all(Option::is_none) {
            return None;
        }
        ResponsiveRule::from_buckets(
            ResponsiveContainer::Nearest {
                name: name.map(str::to_owned),
            },
            axis,
            breakpoints,
            variants,
        )
    }

    #[cfg(test)]
    fn compositions(&self) -> usize {
        self.composed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |composed| composed.compositions)
    }

    #[cfg(test)]
    fn composed_entries(&self) -> usize {
        self.composed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |composed| composed.by_base.len())
    }

    fn hot_rule_matches(meta: HotSiteMeta, active: u64, rule: &HotStyleRule) -> bool {
        let active_pseudo = (active >> PSEUDO_BIT_SHIFT) as u8;
        if rule.pseudo & !active_pseudo != 0 {
            return false;
        }
        rule.classes.iter().all(|class| {
            meta.fixed_names.contains(&class.as_str())
                || meta
                    .conditional_names
                    .iter()
                    .enumerate()
                    .any(|(bit, name)| *name == class.as_str() && (active & (1u64 << bit)) != 0)
        })
    }

    /// `base` (Layer 0) -> matching stylesheet rules (Layer 1) -> optional
    /// author inline overrides (Layer 2), caching composed `Arc<LayoutStyle>`
    /// across instances with identical `(base, active, media, inline)`.
    pub(crate) fn compose_layered(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        inline: Option<&InlineLayer>,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let Some(inline) = inline.filter(|inline| !inline.0.is_empty()) else {
            return self.compose_with_media(base, active, env);
        };
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let inline = composed.canonical_inline_ref(inline);
        let layout = self.compose_layered_in(composed, &base, active, &inline, env);
        composed.prune();
        layout
    }

    /// Like [`Self::compose_layered`], using an already-canonicalized Layer 2
    /// `Arc<InlineLayer>` for pointer-fast cache hits.
    pub(crate) fn compose_layered_arc(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        inline: Option<&Arc<InlineLayer>>,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let Some(inline) = inline.filter(|inline| !inline.0.is_empty()) else {
            return self.compose_with_media(base, active, env);
        };
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let inline = composed.canonical_inline_arc(inline);
        let layout = self.compose_layered_in(composed, &base, active, &inline, env);
        composed.prune();
        layout
    }

    fn compose_layered_in(
        &self,
        composed: &mut Composed,
        base: &Arc<LayoutStyle>,
        active: u64,
        inline: &Arc<InlineLayer>,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let media_mask = self.eval_media_mask(env);
        if let Some(media_mask) = media_mask {
            let key = (
                Arc::as_ptr(base) as usize,
                active,
                media_mask,
                Arc::as_ptr(inline) as usize,
            );
            if let Some((known_base, known_inline, layout)) = composed.by_inline.get(&key)
                && known_base
                    .upgrade()
                    .is_some_and(|known| Arc::ptr_eq(&known, base))
                && known_inline
                    .upgrade()
                    .is_some_and(|known| Arc::ptr_eq(&known, inline))
            {
                return Arc::clone(layout);
            }
            let layer01 = self.compose_in(composed, base, active, env);
            let mut layout = (*layer01).clone();
            inline.apply(&mut layout);
            layout.drop_inert_physical_edges();
            let layout = Arc::new(layout);
            composed.by_inline.insert(
                key,
                (
                    Arc::downgrade(base),
                    Arc::downgrade(inline),
                    Arc::clone(&layout),
                ),
            );
            return layout;
        }
        let layer01 = self.compose_in(composed, base, active, env);
        let mut layout = (*layer01).clone();
        inline.apply(&mut layout);
        layout.drop_inert_physical_edges();
        Arc::new(layout)
    }

    /// [`Self::responsive`] with optional Layer 2 author inline overrides
    /// applied on top of both the base composition and every `@container`
    /// bucket so `@container` (Layer 1) never overrides Layer 2.
    pub(crate) fn responsive_layered(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        inline: Option<&InlineLayer>,
        env: Option<&MediaEnv>,
        at: &'static Location<'static>,
    ) -> Option<Arc<ResponsiveRule>> {
        let Some(inline) = inline else {
            return self.responsive_with_media(base, active, env, at);
        };
        if !self.has_queries() {
            return None;
        }
        let applying: Vec<&SiteRule> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(idx, (needs, _, _))| {
                needs & !active == 0 && self.rule_media_matches(*idx, env)
            })
            .map(|(_, rule)| rule)
            .collect();
        let queries: Vec<&SheetQuery> = applying.iter().filter_map(|rule| rule.2).collect();
        let (name, axis, breakpoints) = plan(&queries, at)?;
        let authored = self.compose_layered(base, active, Some(inline), env);
        let variants: Vec<Option<StyleVariant>> = (0..=breakpoints.len())
            .map(|bucket| {
                let lower = bucket
                    .checked_sub(1)
                    .map_or(f32::NEG_INFINITY, |below| breakpoints[below]);
                let mut styled = apply(
                    base,
                    applying
                        .iter()
                        .filter(|(_, _, query)| query.is_none_or(|query| query.holds_at(lower)))
                        .map(|(_, patch, _)| *patch),
                );
                inline.apply(&mut styled);
                styled.drop_inert_physical_edges();
                StyleVariant::between(&authored, &styled)
            })
            .collect();
        if variants.iter().all(Option::is_none) {
            return None;
        }
        ResponsiveRule::from_buckets(
            ResponsiveContainer::Nearest {
                name: name.map(str::to_owned),
            },
            axis,
            breakpoints,
            variants,
        )
        .map(Arc::new)
    }

    fn compose_hot(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        sheet: &HotStyleSheet,
        inline: Option<&InlineLayer>,
        env: Option<&MediaEnv>,
    ) -> Arc<LayoutStyle> {
        let Some(meta) = self.hot else {
            return self.compose_layered(base, active, inline, env);
        };
        let mut layout = (**base).clone();
        for rule in &sheet.rules {
            if rule.query.is_none() && Self::hot_rule_matches(meta, active, rule) {
                rule.patch.apply(&mut layout);
            }
        }
        if let Some(inline) = inline {
            inline.apply(&mut layout);
        }
        layout.drop_inert_physical_edges();
        Arc::new(layout)
    }

    fn responsive_hot(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        sheet: &HotStyleSheet,
        inline: Option<&InlineLayer>,
        at: &'static Location<'static>,
    ) -> Option<Arc<ResponsiveRule>> {
        let meta = self.hot?;
        let applying: Vec<&HotStyleRule> = sheet
            .rules
            .iter()
            .filter(|rule| Self::hot_rule_matches(meta, active, rule))
            .collect();
        let queries: Vec<&HotSheetQuery> = applying
            .iter()
            .filter_map(|rule| rule.query.as_ref())
            .collect();
        let first = queries.first()?;
        let mut breakpoints: Vec<f32> = queries
            .iter()
            .flat_map(|q| q.intervals.iter())
            .flat_map(|&(lo, hi)| [lo, hi])
            .filter(|bound| bound.is_finite())
            .collect();
        breakpoints.compact_sort_by(f32::total_cmp);
        breakpoints.dedup();
        let why = if queries
            .iter()
            .any(|q| q.name != first.name || q.axis != first.axis)
        {
            Some("ask more than one container or axis")
        } else if breakpoints.len() > MAX_RESPONSIVE_BREAKPOINTS {
            Some("use more breakpoints than one responsive rule holds")
        } else {
            None
        };
        if let Some(why) = why {
            nana_diagnostics::fault!(
                nana_diagnostics::framework::runtime::VIEW_CONTAINER_QUERY_UNSUPPORTED,
                breakpoints = breakpoints.len() as u64;
                "the @container rules of the element whose classes are given at {at} {why}: none of \
                 them apply"
            );
            return None;
        }
        let authored = self.compose_hot(base, active, sheet, inline, None);
        let variants: Vec<Option<StyleVariant>> = (0..=breakpoints.len())
            .map(|bucket| {
                let lower = bucket
                    .checked_sub(1)
                    .map_or(f32::NEG_INFINITY, |below| breakpoints[below]);
                let mut styled = (**base).clone();
                for rule in &applying {
                    if rule.query.as_ref().is_none_or(|q| q.holds_at(lower)) {
                        rule.patch.apply(&mut styled);
                    }
                }
                if let Some(inline) = inline {
                    inline.apply(&mut styled);
                }
                styled.drop_inert_physical_edges();
                StyleVariant::between(&authored, &styled)
            })
            .collect();
        if variants.iter().all(Option::is_none) {
            return None;
        }
        let axis = match first.axis {
            HotSheetAxis::Inline => ResponsiveAxis::Inline,
            HotSheetAxis::Block => ResponsiveAxis::Block,
            HotSheetAxis::Width => ResponsiveAxis::Width,
            HotSheetAxis::Height => ResponsiveAxis::Height,
        };
        ResponsiveRule::from_buckets(
            ResponsiveContainer::Nearest {
                name: first.name.clone(),
            },
            axis,
            breakpoints,
            variants,
        )
        .map(Arc::new)
    }
}

/// A stylesheet compiled at build time (`stylesheet!`, a view's `<style>`):
/// its class rules, transitions, keyframe animations, `@media` rule guards,
/// and `@font-face` declarations, each in cascade order.
pub struct Sheet {
    rules: &'static [SheetRule],
    transitions: &'static [SheetTransition],
    animations: &'static [SheetAnimation],
    /// Sparse `(rule_index, &'static SheetMediaQuery)` pairs guarding entries
    /// of [`Self::rules`].
    rule_media: &'static [(usize, &'static SheetMediaQuery)],
    /// Compiled `@font-face` declarations attached to this sheet.
    font_faces: Option<fn() -> Vec<FontFaceRule>>,
    hot: Option<HotSheetMeta>,
    /// `(fixed classes, conditional classes) -> (site, static animations)`.
    resolved: Mutex<Vec<Resolved>>,
}

type Resolved = (Vec<u16>, Vec<u16>, &'static StyleSite, &'static [Implicit]);

/// One rule: the classes its selector needs, its subject pseudo bitmask,
/// its patch, and the `@container` query of the block it is in (`None` outside one).
#[doc(hidden)]
pub struct SheetRule {
    pub classes: &'static [u16],
    pub pseudo: u8,
    pub patch: &'static StylePatch,
    pub query: Option<&'static SheetQuery>,
}

/// One `transition`: the classes its selector needs, its pseudo bitmask, and what it animates.
#[doc(hidden)]
pub struct SheetTransition {
    pub classes: &'static [u16],
    pub pseudo: u8,
    pub animate: &'static [Implicit],
}

/// One `animation` binding: the classes its selector needs, its pseudo bitmask, and its keyframes.
#[doc(hidden)]
pub struct SheetAnimation {
    pub classes: &'static [u16],
    pub pseudo: u8,
    pub animations: &'static [KeyframeAnimation],
}

impl Sheet {
    #[doc(hidden)]
    pub const fn new(rules: &'static [SheetRule], transitions: &'static [SheetTransition]) -> Self {
        Self {
            rules,
            transitions,
            animations: &[],
            rule_media: &[],
            font_faces: None,
            hot: None,
            resolved: Mutex::new(Vec::new()),
        }
    }

    #[doc(hidden)]
    pub const fn with_animations(
        rules: &'static [SheetRule],
        transitions: &'static [SheetTransition],
        animations: &'static [SheetAnimation],
    ) -> Self {
        Self {
            rules,
            transitions,
            animations,
            rule_media: &[],
            font_faces: None,
            hot: None,
            resolved: Mutex::new(Vec::new()),
        }
    }

    #[doc(hidden)]
    pub const fn with_hot(
        rules: &'static [SheetRule],
        transitions: &'static [SheetTransition],
        animations: &'static [SheetAnimation],
        view: &'static str,
        classes: &'static [&'static str],
    ) -> Self {
        Self {
            rules,
            transitions,
            animations,
            rule_media: &[],
            font_faces: None,
            hot: Some(HotSheetMeta { view, classes }),
            resolved: Mutex::new(Vec::new()),
        }
    }

    /// Attach `@media` query guards to rules of this sheet by rule index.
    #[doc(hidden)]
    pub const fn with_media(
        mut self,
        rule_media: &'static [(usize, &'static SheetMediaQuery)],
    ) -> Self {
        self.rule_media = rule_media;
        self
    }

    /// Attach compiled `@font-face` declarations to this sheet so mounting any
    /// element styled by it automatically registers them on [`crate::UiWorld`].
    #[doc(hidden)]
    pub const fn with_font_faces(mut self, font_faces: fn() -> Vec<FontFaceRule>) -> Self {
        self.font_faces = Some(font_faces);
        self
    }

    /// Whether this sheet declares any `@font-face` rules.
    pub const fn has_font_faces(&self) -> bool {
        self.font_faces.is_some()
    }

    /// Evaluate and return this sheet's `@font-face` declarations.
    pub fn font_faces(&self) -> Vec<FontFaceRule> {
        self.font_faces.map_or_else(Vec::new, |build| build())
    }

    /// The rules an element with these classes can match, with the bit of
    /// each conditional class they need, and the winning transition among
    /// those its fixed classes match.
    fn resolve(
        &self,
        fixed: &[u16],
        conditional: &[u16],
    ) -> (&'static StyleSite, &'static [Implicit]) {
        let mut resolved = self.resolved.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, site, animate)) = resolved
            .iter()
            .find(|(f, c, _, _)| f.as_slice() == fixed && c.as_slice() == conditional)
        {
            return (site, animate);
        }
        let bit = |class: &u16| {
            conditional
                .iter()
                .position(|c| c == class)
                .map_or(0, |at| 1u64 << at)
        };
        let mask_for = |classes: &[u16], pseudo: u8| -> u64 {
            let class_bits = classes
                .iter()
                .filter(|class| !fixed.contains(class))
                .fold(0u64, |needs, class| needs | bit(class));
            class_bits | (u64::from(pseudo) << PSEUDO_BIT_SHIFT)
        };
        let mut used_pseudos = 0u8;
        let mut rules: Vec<SiteRule> = Vec::new();
        let mut rule_media_vec: Vec<Option<&'static SheetMediaQuery>> = Vec::new();
        let mut any_media = false;
        for (rule_idx, rule) in self.rules.iter().enumerate() {
            if !rule
                .classes
                .iter()
                .all(|class| fixed.contains(class) || conditional.contains(class))
            {
                continue;
            }
            used_pseudos |= rule.pseudo;
            rules.push((mask_for(rule.classes, rule.pseudo), rule.patch, rule.query));
            let media = self
                .rule_media
                .iter()
                .find(|(idx, _)| *idx == rule_idx)
                .map(|(_, mq)| *mq);
            if media.is_some() {
                any_media = true;
            }
            rule_media_vec.push(media);
        }
        let site_rule_media: &'static [Option<&'static SheetMediaQuery>] = if any_media {
            Box::leak(rule_media_vec.into_boxed_slice())
        } else {
            &[]
        };
        let transitions: Vec<SiteTransition> = self
            .transitions
            .iter()
            .filter(|t| {
                t.classes
                    .iter()
                    .all(|class| fixed.contains(class) || conditional.contains(class))
            })
            .map(|t| {
                used_pseudos |= t.pseudo;
                (mask_for(t.classes, t.pseudo), t.animate)
            })
            .collect();
        let animations: Vec<SiteAnimation> = self
            .animations
            .iter()
            .filter(|a| {
                a.classes
                    .iter()
                    .all(|class| fixed.contains(class) || conditional.contains(class))
            })
            .map(|a| {
                used_pseudos |= a.pseudo;
                (mask_for(a.classes, a.pseudo), a.animations)
            })
            .collect();
        let has_dynamic_transitions = transitions.iter().any(|(needs, _)| *needs != 0);
        let static_animate = if !has_dynamic_transitions && animations.is_empty() {
            self.transitions
                .iter()
                .rfind(|t| t.pseudo == 0 && t.classes.iter().all(|class| fixed.contains(class)))
                .map_or(&[][..], |t| t.animate)
        } else {
            &[][..]
        };
        let dynamic_transitions: &'static [SiteTransition] =
            if has_dynamic_transitions || !animations.is_empty() {
                Box::leak(transitions.into_boxed_slice())
            } else {
                &[]
            };
        let dynamic_animations: &'static [SiteAnimation] = if animations.is_empty() {
            &[]
        } else {
            Box::leak(animations.into_boxed_slice())
        };
        let hot_site = self.hot.map(|meta| {
            let name_of = |idx: &u16| meta.classes.get(usize::from(*idx)).copied().unwrap_or("");
            let fixed_names: &'static [&'static str] = Box::leak(
                fixed
                    .iter()
                    .map(name_of)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            let conditional_names: &'static [&'static str] = Box::leak(
                conditional
                    .iter()
                    .map(name_of)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            HotSiteMeta {
                view: meta.view,
                fixed_names,
                conditional_names,
            }
        });
        // One per distinct class set an element is written with: bounded
        // by the program's source, not by how many nodes it builds.
        let site: &'static StyleSite = Box::leak(Box::new(StyleSite::with_hot(
            Box::leak(rules.into_boxed_slice()),
            site_rule_media,
            dynamic_transitions,
            dynamic_animations,
            used_pseudos,
            hot_site,
        )));
        resolved.push((fixed.to_vec(), conditional.to_vec(), site, static_animate));
        (site, static_animate)
    }
}

/// One class of a [`Sheet`], as `stylesheet!` names it: `styles::card`.
#[derive(Clone, Copy)]
pub struct Class {
    sheet: &'static Sheet,
    index: u16,
}

impl Class {
    #[doc(hidden)]
    pub const fn new(sheet: &'static Sheet, index: u16) -> Self {
        Self { sheet, index }
    }
}

/// One Layer 2 author inline override (`.css(css!{...})` or static Rust
/// layout/paint builder override). Applied after Layer 1 stylesheet rules and
/// before Layer 3 reactive Rust prop bindings.
#[derive(Clone)]
pub(crate) enum InlineOverride {
    Site(&'static StyleSite),
    Patch(&'static StylePatch),
    Gap(nana_ui_core::LengthSpec),
    Padding(Option<nana_ui_core::LengthSpec>),
    Margin(Option<nana_ui_core::LengthSpec>),
    Width(Option<nana_ui_core::LengthSpec>),
    Height(Option<nana_ui_core::LengthSpec>),
    MinWidth(Option<nana_ui_core::LengthSpec>),
    MaxWidth(Option<nana_ui_core::LengthSpec>),
    MinHeight(Option<nana_ui_core::LengthSpec>),
    MaxHeight(Option<nana_ui_core::LengthSpec>),
    FlexGrow(Option<f32>),
    FlexShrink(Option<f32>),
    Direction(Option<nana_ui_core::FlexDirection>),
    Opacity(f32),
    Transform(Option<nana_ui_core::PaintTransform>),
    Fill(Option<nana_ui_core::PaintColor>),
    Ink(Option<nana_ui_core::PaintColor>),
    Edge(Option<nana_ui_core::PaintColor>),
    Elevation(Option<nana_ui_core::ElevationRole>),
    Hidden(bool),
}

impl InlineOverride {
    fn apply(&self, layout: &mut LayoutStyle) {
        match self {
            Self::Site(site) => {
                for &(needs, patch, query) in site.rules {
                    if needs == 0 && query.is_none() {
                        patch.apply(layout);
                    }
                }
            }
            Self::Patch(patch) => patch.apply(layout),
            Self::Gap(gap) => {
                layout.gap = Some(*gap);
                layout.row_gap = None;
                layout.column_gap = None;
            }
            Self::Padding(padding) => {
                layout.padding = *padding;
                if padding.is_some() {
                    layout.padding_top = None;
                    layout.padding_right = None;
                    layout.padding_bottom = None;
                    layout.padding_left = None;
                    if !layout.logical_edges.is_shared_default() {
                        layout.logical_edges.padding = Default::default();
                    }
                }
            }
            Self::Margin(margin) => {
                layout.margin = *margin;
                if margin.is_some() {
                    layout.margin_top = None;
                    layout.margin_right = None;
                    layout.margin_bottom = None;
                    layout.margin_left = None;
                    if !layout.logical_edges.is_shared_default() {
                        layout.logical_edges.margin = Default::default();
                    }
                }
            }
            Self::Width(v) => layout.width = *v,
            Self::Height(v) => layout.height = *v,
            Self::MinWidth(v) => layout.min_width = *v,
            Self::MaxWidth(v) => layout.max_width = *v,
            Self::MinHeight(v) => layout.min_height = *v,
            Self::MaxHeight(v) => layout.max_height = *v,
            Self::FlexGrow(v) => layout.flex_grow = *v,
            Self::FlexShrink(v) => layout.flex_shrink = *v,
            Self::Direction(v) => layout.direction = *v,
            Self::Opacity(v) => layout.opacity = Some(*v),
            Self::Transform(v) => {
                if layout.transforms.transform != *v {
                    layout.transforms.transform = *v;
                }
            }
            Self::Fill(v) => {
                if layout.paint_colors.background != *v {
                    layout.paint_colors.background = *v;
                }
            }
            Self::Ink(v) => {
                if layout.paint_colors.color != *v {
                    layout.paint_colors.color = *v;
                }
            }
            Self::Edge(v) => {
                if layout.paint_colors.border != *v {
                    layout.paint_colors.border = *v;
                }
            }
            Self::Elevation(v) => {
                if layout.paint.elevation != *v {
                    layout.paint.elevation = *v;
                }
            }
            Self::Hidden(v) => layout.hidden = *v,
        }
    }
}

impl PartialEq for InlineOverride {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Site(a), Self::Site(b)) => std::ptr::eq(*a, *b),
            (Self::Patch(a), Self::Patch(b)) => std::ptr::eq(*a, *b),
            (Self::Gap(a), Self::Gap(b)) => a == b,
            (Self::Padding(a), Self::Padding(b)) => a == b,
            (Self::Margin(a), Self::Margin(b)) => a == b,
            (Self::Width(a), Self::Width(b)) => a == b,
            (Self::Height(a), Self::Height(b)) => a == b,
            (Self::MinWidth(a), Self::MinWidth(b)) => a == b,
            (Self::MaxWidth(a), Self::MaxWidth(b)) => a == b,
            (Self::MinHeight(a), Self::MinHeight(b)) => a == b,
            (Self::MaxHeight(a), Self::MaxHeight(b)) => a == b,
            (Self::FlexGrow(a), Self::FlexGrow(b)) => a.map(f32::to_bits) == b.map(f32::to_bits),
            (Self::FlexShrink(a), Self::FlexShrink(b)) => {
                a.map(f32::to_bits) == b.map(f32::to_bits)
            }
            (Self::Direction(a), Self::Direction(b)) => a == b,
            (Self::Opacity(a), Self::Opacity(b)) => a.to_bits() == b.to_bits(),
            (Self::Transform(a), Self::Transform(b)) => a == b,
            (Self::Fill(a), Self::Fill(b)) => a == b,
            (Self::Ink(a), Self::Ink(b)) => a == b,
            (Self::Edge(a), Self::Edge(b)) => a == b,
            (Self::Elevation(a), Self::Elevation(b)) => a == b,
            (Self::Hidden(a), Self::Hidden(b)) => a == b,
            _ => false,
        }
    }
}

/// Ordered Layer 2 overrides attached to an element.
#[derive(Default, Clone, PartialEq)]
pub(crate) struct InlineLayer(Vec<InlineOverride>);

impl InlineLayer {
    pub(crate) fn push(&mut self, item: InlineOverride) {
        self.0.push(item);
    }

    pub(crate) fn apply(&self, layout: &mut LayoutStyle) {
        for item in &self.0 {
            item.apply(layout);
        }
    }

    /// Apply this Layer 2 stack over `base`, preserving `StyleSite` `Arc`
    /// sharing when there is a single compiled `css! { ... }` site.
    pub(crate) fn apply_over(&self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        if let [InlineOverride::Site(site)] = self.0.as_slice() {
            return site.compose(base, 0);
        }
        let mut layout = (**base).clone();
        self.apply(&mut layout);
        layout.drop_inert_physical_edges();
        Arc::new(layout)
    }
}

/// Type-erased Layer 2 carrier stored on [`El`] so `El<C>::place_node` can
/// apply Layer 2 even without `C: StyledComponent` in its generic bound.
pub(crate) struct InlineStyles<C> {
    pub(crate) layer: InlineLayer,
    apply: fn(&mut C, InlineLayer),
}

impl<C> InlineStyles<C> {
    pub(crate) fn new(apply: fn(&mut C, InlineLayer)) -> Self {
        Self {
            layer: InlineLayer::default(),
            apply,
        }
    }

    pub(crate) fn apply_direct(self, component: &mut C) {
        (self.apply)(component, self.layer);
    }
}

pub(crate) fn apply_inline_only<C: StyledComponent>(component: &mut C, inline: InlineLayer) {
    let composed = inline.apply_over(&component.node_style().layout);
    component.node_style_mut().layout = composed;
}

/// The classes [`El::class`] and [`El::class_when`] gave an element.
pub(crate) struct Classes<C> {
    sheet: &'static Sheet,
    fixed: Vec<u16>,
    conditional: Vec<(u16, PropSource<bool>)>,
    apply: ApplySite<C>,
    at: &'static Location<'static>,
}

type ApplySite<C> = fn(
    &mut C,
    &mut NodeBindings<C>,
    &'static StyleSite,
    Vec<PropSource<bool>>,
    Option<InlineLayer>,
    MediaEnv,
    &mut Vec<Signal<MediaEnv>>,
    &'static Location<'static>,
);

impl<C> Classes<C> {
    /// The stylesheet these classes belong to.
    pub(crate) fn sheet(&self) -> &'static Sheet {
        self.sheet
    }

    /// Write the element's styles into it (Layer 0 -> Layer 1 -> Layer 2), and
    /// return its static animations.
    pub(crate) fn apply(
        self,
        component: &mut C,
        bindings: &mut NodeBindings<C>,
        inline: Option<InlineLayer>,
        media_env: MediaEnv,
        media_signals: &mut Vec<Signal<MediaEnv>>,
    ) -> &'static [Implicit] {
        let indices: Vec<u16> = self.conditional.iter().map(|(class, _)| *class).collect();
        let (site, animate) = self.sheet.resolve(&self.fixed, &indices);
        let sources = self
            .conditional
            .into_iter()
            .map(|(_, source)| source)
            .collect();
        (self.apply)(
            component,
            bindings,
            site,
            sources,
            inline,
            media_env,
            media_signals,
            self.at,
        );
        animate
    }
}

/// Dynamic Layer 0+1+2 style composition binding installed into
/// `NodeBindings::composed_style`, which always runs BEFORE Layer 3
/// (`direct` and `dynamic`) reactive bindings.
struct ComposedStyleProp<F> {
    f: F,
    last: Option<Arc<LayoutStyle>>,
    pending: Option<Arc<LayoutStyle>>,
}

impl<C: StyledComponent, F: Fn() -> Arc<LayoutStyle> + Send> DynProp<C> for ComposedStyleProp<F> {
    fn evaluate(&mut self, _current: &C) -> bool {
        let next = (self.f)();
        let changed = match &self.last {
            Some(prev) => !Arc::ptr_eq(prev, &next) && **prev != *next,
            None => true,
        };
        if changed {
            self.pending = Some(next);
        }
        changed
    }

    fn write(&mut self, target: &mut C) {
        if let Some(next) = self.pending.take() {
            self.last = Some(Arc::clone(&next));
            target.node_style_mut().layout = next;
        }
    }

    fn discard(&mut self) {
        self.pending = None;
    }
}

/// Per-node interactive pseudo state binding driven by [`ReactiveHost`]:
/// tracks the node's own `:hover`/`:active`/`:focus`/`:focus-visible`/`:selected`
/// stylesheet signal plus any ancestor [`InteractiveStateHandle`] observers.
#[derive(Clone)]
pub(crate) struct InteractiveStyleBinding {
    pub(crate) used_pseudos: u8,
    pub(crate) signal: Option<Signal<u8>>,
    pub(crate) observers: Vec<(u8, Signal<u8>)>,
}

impl InteractiveStyleBinding {
    pub(crate) fn bind_self(slot: &mut Option<Self>, used_pseudos: u8, sig: Signal<u8>) {
        match slot {
            Some(existing) => {
                existing.used_pseudos |= used_pseudos;
                existing.signal = Some(sig);
            }
            None => {
                *slot = Some(Self {
                    used_pseudos,
                    signal: Some(sig),
                    observers: Vec::new(),
                });
            }
        }
    }

    pub(crate) fn add_observer(slot: &mut Option<Self>, mask: u8, sig: Signal<u8>) {
        match slot {
            Some(existing) => {
                existing.used_pseudos |= mask;
                existing.observers.push((mask, sig));
            }
            None => {
                *slot = Some(Self {
                    used_pseudos: mask,
                    signal: None,
                    observers: vec![(mask, sig)],
                });
            }
        }
    }
}

/// Per-node dynamic motion metadata (conditional/pseudo transitions & keyframes).
#[derive(Clone)]
pub(crate) struct MotionBinding {
    pub(crate) site: &'static StyleSite,
    pub(crate) active_mask: Arc<AtomicU64>,
    pub(crate) prev_mask: u64,
    /// Sentinel `u64::MAX` ensures initial mount syncs static `@keyframes` once.
    pub(crate) last_anim_mask: u64,
}

impl MotionBinding {
    pub(crate) fn current_mask(&self) -> u64 {
        self.active_mask.load(Ordering::Relaxed)
    }
}

/// `site` over the component's Layer 0 layout, followed by optional Layer 2
/// author inline overrides; the `i`-th condition switches the site's `i`-th
/// conditional class. A site with `@container` rules also gives the element
/// its responsive rule under the active classes, kept in `bindings` for the
/// node to send.
#[allow(clippy::too_many_arguments)]
fn apply_site<C: StyledComponent + ComponentView>(
    component: &mut C,
    bindings: &mut NodeBindings<C>,
    site: &'static StyleSite,
    conditions: Vec<PropSource<bool>>,
    inline: Option<InlineLayer>,
    media_env: MediaEnv,
    media_signals: &mut Vec<Signal<MediaEnv>>,
    at: &'static Location<'static>,
) {
    let base = &component.node_style().layout;
    let queried = site.has_queries();
    let has_media = site.has_media();
    let used_pseudos = site.used_pseudos();
    let hot_styles = site
        .hot
        .and_then(|meta| super::hot::hot_styles_signal(meta.view));
    let motion_mask = if site.has_dynamic_motion() {
        let active_mask = Arc::new(AtomicU64::new(0));
        let mask_clone = Arc::clone(&active_mask);
        bindings.motion = Some(MotionBinding {
            site,
            active_mask,
            prev_mask: 0,
            last_anim_mask: u64::MAX,
        });
        Some(mask_clone)
    } else {
        None
    };
    if conditions.is_empty() && used_pseudos == 0 && hot_styles.is_none() && !has_media {
        if queried {
            bindings.responsive = Some(ResponsiveBinding::new(site.responsive_layered(
                base,
                0,
                inline.as_ref(),
                None,
                at,
            )));
        }
        let composed = site.compose_layered(base, 0, inline.as_ref(), None);
        <ComposedLayout as FieldWrite<C, Arc<LayoutStyle>>>::write(component, composed);
        return;
    }
    let pseudo_signal = if used_pseudos != 0 {
        let sig = signal(0u8);
        InteractiveStyleBinding::bind_self(&mut bindings.interactive, used_pseudos, sig);
        Some(sig)
    } else {
        None
    };
    let media_signal = if has_media {
        let sig = signal(media_env);
        media_signals.push(sig);
        Some(sig)
    } else {
        None
    };
    let base = site.shared_base(base);
    let inline = inline.map(|inline| site.shared_inline(inline));
    // The binding computes the active classes and pseudo bits once per run, for both.
    let pending = (queried || hot_styles.is_some()).then(|| {
        let binding = ResponsiveBinding::default();
        let pending = Arc::clone(&binding.pending);
        bindings.responsive = Some(binding);
        pending
    });
    let composed = move || {
        let mut active = conditions
            .iter()
            .enumerate()
            .fold(0u64, |mask, (bit, class)| match class.get() {
                true => mask | (1u64 << bit),
                false => mask,
            });
        if let Some(pseudo) = pseudo_signal {
            active |= u64::from(pseudo.get()) << PSEUDO_BIT_SHIFT;
        }
        if let Some(mask) = &motion_mask {
            mask.store(active, Ordering::Relaxed);
        }
        let env = media_signal.map(|sig| sig.get());
        if let Some(hot_sheet) = hot_styles.and_then(|sig| sig.get()) {
            if let Some(pending) = &pending {
                let rule = site.responsive_hot(&base, active, &hot_sheet, inline.as_deref(), at);
                *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(rule);
            }
            site.compose_hot(&base, active, &hot_sheet, inline.as_deref(), env.as_ref())
        } else {
            if let Some(pending) = &pending {
                let rule = if queried {
                    site.responsive_layered(&base, active, inline.as_deref(), env.as_ref(), at)
                } else {
                    None
                };
                *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(rule);
            }
            site.compose_layered_arc(&base, active, inline.as_ref(), env.as_ref())
        }
    };
    bindings.composed_style = Some(DynBinding {
        prop: Box::new(ComposedStyleProp {
            f: composed,
            last: None,
            pending: None,
        }),
        field: <ComposedLayout as FieldWrite<C, Arc<LayoutStyle>>>::FIELD,
        site: at,
    });
}

/// The responsive rule an element's compiled `@container` rules give it,
/// sent beside its other changes: in the commit that builds the node, then
/// whenever the active classes give another one.
#[derive(Default)]
pub(crate) struct ResponsiveBinding {
    /// What the last evaluation of the element's classes gave and is not
    /// sent yet; shared with that binding.
    pending: Arc<Mutex<Option<Option<Arc<ResponsiveRule>>>>>,
    /// What was sent last.
    sent: Option<Arc<ResponsiveRule>>,
}

impl ResponsiveBinding {
    fn new(rule: Option<Arc<ResponsiveRule>>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(Some(rule))),
            sent: None,
        }
    }

    /// The rule to send in the commit that creates the node: what the
    /// first evaluation gave, the absence of a rule included, so a node
    /// built again over one that had a rule loses it.
    pub(crate) fn initial(&mut self) -> Option<Option<Arc<ResponsiveRule>>> {
        let rule = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        self.sent.clone_from(&rule);
        Some(rule)
    }

    /// Queue the rule the last evaluation gave when it differs from the one
    /// sent. A cached rule is the same `Arc`, so a run that keeps the active
    /// classes compares one pointer.
    pub(crate) fn send(&mut self, node: StableNodeId, mutations: &mut MutationQueue) {
        let Some(rule) = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let same = match (&rule, &self.sent) {
            (Some(rule), Some(sent)) => Arc::ptr_eq(rule, sent) || rule == sent,
            (None, None) => true,
            _ => false,
        };
        if !same {
            mutations.set_responsive(node, rule.clone());
            self.sent = rule;
        }
    }
}

fn apply(base: &LayoutStyle, patches: impl Iterator<Item = &'static StylePatch>) -> LayoutStyle {
    let mut layout = base.clone();
    for patch in patches {
        patch.apply(&mut layout);
    }
    // Every patch is in, so the cascade is settled: a physical edge record
    // with no logical edge to order against decides nothing, and dropping it
    // gives the box the intent Rust states for the same edges.
    layout.drop_inert_physical_edges();
    layout
}

/// Writes a composed layout (Layer 0 -> Layer 1 -> Layer 2).
/// Layer 3 reactive prop bindings (including `.visible(sig)`) run after this
/// writer in `NodeBindings`.
#[doc(hidden)]
pub struct ComposedLayout;

impl<C: StyledComponent> FieldWrite<C, Arc<LayoutStyle>> for ComposedLayout {
    const FIELD: &'static str = "style.layout";

    fn write(target: &mut C, layout: Arc<LayoutStyle>) {
        target.node_style_mut().layout = layout;
    }

    fn differs(target: &C, layout: &Arc<LayoutStyle>) -> bool {
        let current = &target.node_style().layout;
        !Arc::ptr_eq(current, layout) && **current != **layout
    }
}

impl<C: StyledComponent + ComponentView, K> El<C, K> {
    /// Give the element `class` of a stylesheet:
    ///
    /// ```ignore
    /// stylesheet! {
    ///     mod styles;
    ///     .card { padding: 12px; }
    ///     .card.done { opacity: 0.5; }
    /// }
    /// widget(card).class(styles::card).class_when(styles::done, done)
    /// ```
    ///
    /// Every class of one element comes from one sheet. Its rules apply
    /// over the layout the element was built with, in the sheet's cascade
    /// order; a `transition` on its fixed classes animates what bindings
    /// change.
    #[track_caller]
    pub fn class(self, class: Class) -> Self {
        self.class_at(class, Location::caller())
    }

    /// `class` while `condition` holds (Vue's `:class="{ done: … }"`).
    #[track_caller]
    pub fn class_when(self, class: Class, condition: impl IntoProp<bool>) -> Self {
        self.class_when_at(class, condition.into_source(), Location::caller())
    }

    /// Enable `class` while the ancestor tracked by `handle` matches every
    /// interactive pseudo-state bit in `pseudo_mask` (e.g. `.card:hover .icon`).
    #[track_caller]
    pub fn ancestor_pseudo(
        self,
        class: Class,
        handle: InteractiveStateHandle,
        pseudo_mask: u8,
    ) -> Self {
        self.class_when_at(
            class,
            (move || handle.matches(pseudo_mask)).into_source(),
            Location::caller(),
        )
    }

    fn class_at(self, class: Class, at: &'static Location<'static>) -> Self {
        self.with_classes(class, at, |classes, index| classes.fixed.push(index))
    }

    fn class_when_at(
        self,
        class: Class,
        condition: PropSource<bool>,
        at: &'static Location<'static>,
    ) -> Self {
        self.with_classes(class, at, move |classes, index| {
            if classes.conditional.len() < MAX_CONDITIONAL_CLASSES {
                classes.conditional.push((index, condition));
            }
        })
    }

    fn with_classes(
        mut self,
        class: Class,
        at: &'static Location<'static>,
        add: impl FnOnce(&mut Classes<C>, u16),
    ) -> Self {
        let classes = self.classes_mut().get_or_insert_with(|| Classes {
            sheet: class.sheet,
            fixed: Vec::new(),
            conditional: Vec::new(),
            apply: apply_site::<C>,
            at,
        });
        assert!(
            std::ptr::eq(classes.sheet, class.sheet),
            "an element's classes come from one stylesheet"
        );
        add(classes, class.index);
        self
    }

    /// Record a Layer 2 author inline override (applied after Layer 1
    /// stylesheet rules and before Layer 3 reactive bindings).
    pub(crate) fn push_inline_override(&mut self, item: InlineOverride) {
        self.inline_styles_mut()
            .get_or_insert_with(|| Box::new(InlineStyles::new(apply_inline_only::<C>)))
            .layer
            .push(item);
    }

    /// Apply a compiled [`StylePatch`] as a Layer 2 author inline override
    /// (wins over `.class` / `.class_when` / `:hover` / `@container`, regardless
    /// of builder call order; yields to Layer 3 reactive prop bindings).
    pub fn inline_patch(mut self, patch: &'static StylePatch) -> Self {
        self.push_inline_override(InlineOverride::Patch(patch));
        self
    }
}

/// One declaration block compiled at build time (`css! { … }`): its patch
/// and its implicit animations. See [`El::css`].
#[derive(Clone, Copy)]
pub struct InlineStyle {
    site: Option<&'static StyleSite>,
    animate: &'static [super::Implicit],
}

impl InlineStyle {
    #[doc(hidden)]
    pub const fn new(
        site: Option<&'static StyleSite>,
        animate: &'static [super::Implicit],
    ) -> Self {
        Self { site, animate }
    }
}

impl<C: StyledComponent + ComponentView, K> El<C, K> {
    /// A declaration block compiled by `css!`: `column().gap(8.0).children(rows)
    /// .css(css! { padding: 12px; transition: opacity 150ms })`.
    ///
    /// Applies as Layer 2 (author inline override): wins over Layer 1
    /// stylesheet rules (`.class`, `.class_when`, `:hover`, `@container`)
    /// regardless of builder call order, while Layer 3 reactive bindings win
    /// over it.
    #[track_caller]
    pub fn css(self, style: InlineStyle) -> Self {
        self.css_at(style, Location::caller())
    }

    fn css_at(mut self, style: InlineStyle, _at: &'static Location<'static>) -> Self {
        if let Some(site) = style.site {
            self.push_inline_override(InlineOverride::Site(site));
        }
        self.animate(style.animate.iter().copied())
    }
}

/// What [`El::class`], [`El::class_when`] and [`El::css`] give the
/// container of a structural view (`each`, `when`, `dynamic`,
/// `each_virtual`), kept until the view builds its container.
#[derive(Default)]
pub(crate) struct ContainerStyle(Vec<ContainerStyleOp>);

enum ContainerStyleOp {
    Class(Class, &'static Location<'static>),
    ClassWhen(Class, PropSource<bool>, &'static Location<'static>),
    Css(InlineStyle, &'static Location<'static>),
    Visible(PropSource<bool>, &'static Location<'static>),
}

impl ContainerStyle {
    /// Whether no style operations were attached to the control-flow node.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `element` with these styles, in the order they were given.
    pub(crate) fn apply<C: StyledComponent + ComponentView>(self, element: El<C>) -> El<C> {
        self.0.into_iter().fold(element, |element, op| match op {
            ContainerStyleOp::Class(class, at) => element.class_at(class, at),
            ContainerStyleOp::ClassWhen(class, condition, at) => {
                element.class_when_at(class, condition, at)
            }
            ContainerStyleOp::Css(style, at) => element.css_at(style, at),
            ContainerStyleOp::Visible(visible, at) => element.visible_at(visible, at),
        })
    }
}

/// `.class`, `.class_when`, `.css` and `.visible` on a structural view: they
/// style the container its rows or branches are built in, as they style an
/// element.
macro_rules! container_styles {
    ($([$($generics:tt)*] $view:ty),* $(,)?) => {$(
        impl<$($generics)*> $view {
            /// Give the container the rows or branches are built in `class`
            /// of a stylesheet, as [`El::class`] gives an element one.
            #[track_caller]
            pub fn class(mut self, class: $crate::view::Class) -> Self {
                self.container_style_mut().push_class(class, ::std::panic::Location::caller());
                self
            }

            /// `class` on the container while `condition` holds.
            #[track_caller]
            pub fn class_when(
                mut self,
                class: $crate::view::Class,
                condition: impl $crate::view::IntoProp<bool>,
            ) -> Self {
                self.container_style_mut().push_class_when(
                    class,
                    condition.into_source(),
                    ::std::panic::Location::caller(),
                );
                self
            }

            /// Enable `class` on the container while the ancestor tracked by
            /// `handle` matches every pseudo bit in `pseudo_mask`.
            #[track_caller]
            pub fn ancestor_pseudo(
                self,
                class: $crate::view::Class,
                handle: $crate::view::InteractiveStateHandle,
                pseudo_mask: u8,
            ) -> Self {
                self.class_when(class, move || handle.matches(pseudo_mask))
            }

            /// A `css!` block on the container, as [`El::css`] on an
            /// element.
            #[track_caller]
            pub fn css(mut self, style: $crate::view::InlineStyle) -> Self {
                self.container_style_mut().push_css(style, ::std::panic::Location::caller());
                self
            }

            /// Keep the container, and what is built in it, but take it out
            /// of layout, paint and hit testing while `visible` is false
            /// (`v-show`), as [`El::visible`] does an element's.
            #[track_caller]
            pub fn visible(mut self, visible: impl $crate::view::IntoProp<bool>) -> Self {
                self.container_style_mut().push_visible(
                    visible.into_source(),
                    ::std::panic::Location::caller(),
                );
                self
            }
        }
    )*};
}
pub(crate) use container_styles;

impl ContainerStyle {
    /// Take out the `.visible` conditions, for a view whose container has a
    /// visibility rule of its own to combine them with.
    pub(crate) fn take_visible(&mut self) -> Vec<PropSource<bool>> {
        let mut visible = Vec::new();
        self.0.retain_mut(|op| match op {
            ContainerStyleOp::Visible(source, _) => {
                visible.push(std::mem::replace(source, PropSource::Const(true)));
                false
            }
            _ => true,
        });
        visible
    }

    pub(crate) fn push_class(&mut self, class: Class, at: &'static Location<'static>) {
        self.0.push(ContainerStyleOp::Class(class, at));
    }

    pub(crate) fn push_class_when(
        &mut self,
        class: Class,
        condition: PropSource<bool>,
        at: &'static Location<'static>,
    ) {
        self.0
            .push(ContainerStyleOp::ClassWhen(class, condition, at));
    }

    pub(crate) fn push_css(&mut self, style: InlineStyle, at: &'static Location<'static>) {
        self.0.push(ContainerStyleOp::Css(style, at));
    }

    pub(crate) fn push_visible(
        &mut self,
        visible: PropSource<bool>,
        at: &'static Location<'static>,
    ) {
        self.0.push(ContainerStyleOp::Visible(visible, at));
    }
}

#[cfg(test)]
mod site_tests {
    use super::*;
    use nana_ui_core::LengthSpec;

    static SITE: StyleSite = StyleSite::new(&[]);
    static SHARED: StyleSite = StyleSite::new(&[]);

    /// Equal bases at different addresses share one composition; bases no
    /// instance holds any more do not keep theirs alive for ever.
    #[test]
    fn a_site_shares_equal_bases_and_forgets_dropped_ones() {
        let one = Arc::new(LayoutStyle::default());
        let other = Arc::new(LayoutStyle::default());
        assert!(Arc::ptr_eq(
            &SHARED.compose(&one, 0),
            &SHARED.compose(&other, 0)
        ));

        for width in 0..500 {
            let base = Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(width as f32)),
                ..LayoutStyle::default()
            });
            let _ = SITE.compose(&base, 0);
        }
        assert!(
            SITE.composed_entries() <= 128,
            "{} entries for bases nothing holds",
            SITE.composed_entries()
        );
    }

    static PAD: StylePatch = StylePatch::new(|layout| {
        layout.padding = Some(LengthSpec::Px(4.0));
    });
    static PADDED: [SheetRule; 1] = [SheetRule {
        classes: &[0],
        pseudo: 0,
        patch: &PAD,
        query: None,
    }];
    static PADDED_SHEET: Sheet = Sheet::new(&PADDED, &[]);

    /// Mounting more of the same styled rows composes nothing new: an
    /// element without conditional classes drops its base for the
    /// composition, and the next equal one still finds it (#348).
    #[test]
    fn equal_elements_compose_once_however_many_mount() {
        use crate::view::{button, column, row, text};
        use crate::{AppContext, DocumentId};
        let class = Class::new(&PADDED_SHEET, 0);
        let rows = |from: usize, count: usize| {
            (from..from + count)
                .map(|i| {
                    row().class(class).children((
                        text(format!("row {i}")).class(class),
                        button(format!("button {i}")).class(class),
                    ))
                })
                .collect::<Vec<_>>()
        };
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let list = cx
            .mount_view_root(document, || column().children(rows(0, 1)))
            .unwrap()
            .roots()[0];
        let (site, _) = PADDED_SHEET.resolve(&[0], &[]);
        let first = site.compositions();
        assert!(first > 0, "the first row composes its elements");
        cx.mount_view(list, || rows(1, 300)).unwrap();
        assert_eq!(
            site.compositions(),
            first,
            "300 more equal rows composed again"
        );
        let padding = |id| cx.world().node_style(id).unwrap().layout.padding;
        let children = cx.world().node(list).unwrap().children.to_vec();
        assert_eq!(children.len(), 301);
        assert!(
            children
                .iter()
                .all(|row| padding(*row) == Some(LengthSpec::Px(4.0)))
        );
    }

    static ROW: StylePatch = StylePatch::new(|layout| {
        layout.height = Some(LengthSpec::Px(20.0));
    });
    static NARROW: StylePatch = StylePatch::new(|layout| {
        layout.height = Some(LengthSpec::Px(40.0));
        layout.opacity = Some(0.5);
    });
    static LATER: StylePatch = StylePatch::new(|layout| {
        layout.height = Some(LengthSpec::Px(30.0));
    });
    /// `@container card (width < 300px)`.
    static CARD: SheetQuery = SheetQuery::new(
        Some("card"),
        ResponsiveAxis::Width,
        &[(f32::NEG_INFINITY, 300.0)],
    );
    static QUERIED: StyleSite = StyleSite::new(&[
        (0, &ROW, None),
        (0, &NARROW, Some(&CARD)),
        (1, &LATER, None),
    ]);

    fn here() -> &'static Location<'static> {
        Location::caller()
    }

    /// Composing leaves a guarded rule out; the responsive rule carries it
    /// in the buckets where its query holds, written over the authored
    /// composition, and a later plain rule still wins over it there.
    #[test]
    fn guarded_rules_become_buckets_and_a_later_plain_rule_still_wins() {
        let base = Arc::new(LayoutStyle::default());
        let authored = QUERIED.compose(&base, 0);
        assert_eq!(authored.height, Some(LengthSpec::Px(20.0)));
        assert_eq!(authored.opacity, None, "the guarded rule is not composed");

        let rule = QUERIED.responsive(&base, 0, here()).expect("a rule");
        assert_eq!(
            rule.container(),
            &ResponsiveContainer::Nearest {
                name: Some("card".into())
            }
        );
        assert_eq!(rule.axis(), ResponsiveAxis::Width);
        assert_eq!(rule.bucket_for(299.9), 0);
        assert_eq!(rule.bucket_for(300.0), 1, "`<` leaves its bound out");
        let mut narrow = (*authored).clone();
        rule.variant(0).expect("below 300").apply(&mut narrow);
        assert_eq!(narrow.height, Some(LengthSpec::Px(40.0)));
        assert_eq!(narrow.opacity, Some(0.5));
        assert!(rule.variant(1).is_none(), "the authored style above");
        assert!(
            Arc::ptr_eq(&rule, &QUERIED.responsive(&base, 0, here()).unwrap()),
            "one rule per base and active set"
        );

        let on = QUERIED.responsive(&base, 1, here()).expect("a rule");
        let mut styled = (*QUERIED.compose(&base, 1)).clone();
        assert_eq!(styled.height, Some(LengthSpec::Px(30.0)));
        on.variant(0).expect("below 300").apply(&mut styled);
        assert_eq!(styled.height, Some(LengthSpec::Px(30.0)), "the later rule");
        assert_eq!(styled.opacity, Some(0.5));
    }

    static ELSEWHERE: SheetQuery =
        SheetQuery::new(None, ResponsiveAxis::Width, &[(f32::NEG_INFINITY, 100.0)]);
    static TWO_CONTAINERS: StyleSite =
        StyleSite::new(&[(0, &NARROW, Some(&CARD)), (1, &LATER, Some(&ELSEWHERE))]);
    static SEVENTEEN: SheetQuery = SheetQuery::new(
        None,
        ResponsiveAxis::Inline,
        &[
            (0.0, 1.0),
            (2.0, 3.0),
            (4.0, 5.0),
            (6.0, 7.0),
            (8.0, 9.0),
            (10.0, 11.0),
            (12.0, 13.0),
            (14.0, 15.0),
            (16.0, f32::INFINITY),
        ],
    );
    static TOO_MANY: StyleSite = StyleSite::new(&[(0, &NARROW, Some(&SEVENTEEN))]);
    static SAME: StyleSite = StyleSite::new(&[(0, &ROW, None), (0, &ROW, Some(&CARD))]);

    /// An element follows one container on one axis, through at most 16
    /// breakpoints; past that its guarded rules make no rule. Nor does a
    /// guarded rule that changes nothing.
    #[test]
    fn guarded_rules_that_cannot_share_one_plan_make_no_rule() {
        let base = Arc::new(LayoutStyle::default());
        assert!(TWO_CONTAINERS.responsive(&base, 0, here()).is_some());
        assert!(
            TWO_CONTAINERS.responsive(&base, 1, here()).is_none(),
            "two containers once the class is on"
        );
        assert!(TOO_MANY.responsive(&base, 0, here()).is_none());
        assert!(SAME.responsive(&base, 0, here()).is_none());
        assert!(
            SITE.responsive(&base, 0, here()).is_none(),
            "no guarded rule"
        );
    }

    static SEL_RESTING: StylePatch = StylePatch::new(|layout| {
        layout.opacity = Some(1.0);
        layout.padding = Some(LengthSpec::Px(4.0));
    });
    static SEL_ACTIVE: StylePatch = StylePatch::new(|layout| {
        layout.opacity = Some(0.6);
        layout.padding = Some(LengthSpec::Px(12.0));
    });
    static SEL_RULES: [SheetRule; 2] = [
        SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &SEL_RESTING,
            query: None,
        },
        SheetRule {
            classes: &[0],
            pseudo: PSEUDO_SELECTED,
            patch: &SEL_ACTIVE,
            query: None,
        },
    ];
    static SEL_SHEET: Sheet = Sheet::new(&SEL_RULES, &[]);

    #[test]
    fn selected_pseudo_class_updates_composed_style_and_aria_selected() {
        use crate::AccessibilityRole;
        use crate::view::{column, list_item, signal};
        use crate::{AppContext, DocumentId};

        let sel_class = Class::new(&SEL_SHEET, 0);
        let item_selected = signal(false);
        let card_selected = signal(false);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                column().children((
                    list_item("alpha").class(sel_class).selected(item_selected),
                    column()
                        .class(sel_class)
                        .aria_selected(move || Some(card_selected.get())),
                ))
            })
            .unwrap()
            .roots()[0];
        let children = cx.world().node(root).unwrap().children.to_vec();
        let (item_id, card_id) = (children[0], children[1]);

        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        assert_eq!(
            cx.world().node_style(item_id).unwrap().layout.opacity,
            Some(1.0)
        );
        assert_eq!(
            cx.world().node_style(card_id).unwrap().layout.opacity,
            Some(1.0)
        );
        let projected = cx.world().project_accessibility(document);
        assert_eq!(
            projected
                .iter()
                .find(|n| n.id == item_id && n.role == AccessibilityRole::ListItem)
                .and_then(|n| n.selected),
            Some(false)
        );
        assert_eq!(
            projected
                .iter()
                .find(|n| n.id == card_id)
                .and_then(|n| n.selected),
            Some(false)
        );

        item_selected.set(true);
        card_selected.set(true);
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        assert_eq!(
            cx.world().node_style(item_id).unwrap().layout.opacity,
            Some(0.6)
        );
        assert_eq!(
            cx.world().node_style(item_id).unwrap().layout.padding,
            Some(LengthSpec::Px(12.0))
        );
        assert_eq!(
            cx.world().node_style(card_id).unwrap().layout.opacity,
            Some(0.6)
        );
        assert_eq!(
            cx.world().node_style(card_id).unwrap().layout.padding,
            Some(LengthSpec::Px(12.0))
        );
        let projected = cx.world().project_accessibility(document);
        assert_eq!(
            projected
                .iter()
                .find(|n| n.id == item_id)
                .and_then(|n| n.selected),
            Some(true)
        );
        assert_eq!(
            projected
                .iter()
                .find(|n| n.id == card_id)
                .and_then(|n| n.selected),
            Some(true)
        );
    }

    static HOT_BASE_PATCH: StylePatch = StylePatch::new(|layout| {
        layout.padding = Some(LengthSpec::Px(4.0));
        layout.opacity = Some(0.8);
    });
    static HOT_RULES_INIT: [SheetRule; 1] = [SheetRule {
        classes: &[0],
        pseudo: 0,
        patch: &HOT_BASE_PATCH,
        query: None,
    }];
    static HOT_SHEET: Sheet =
        Sheet::with_hot(&HOT_RULES_INIT, &[], &[], "HotStyleUnit", &["box", "on"]);

    #[test]
    fn hot_styles_override_revert_defaults_handle_conditional_and_container_queries() {
        use crate::view::{__hot_register, HotStylePatch, apply_hot_styles, column, signal};
        use crate::{AppContext, DocumentId};

        let shape = 0x4220_0001;
        __hot_register("HotStyleUnit", shape);
        let c_box = Class::new(&HOT_SHEET, 0);
        let c_on = Class::new(&HOT_SHEET, 1);
        let flag = signal(false);

        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                column().class(c_box).class_when(c_on, flag)
            })
            .unwrap()
            .roots()[0];

        let layout = |cx: &AppContext| cx.world().node_style(root).unwrap().layout.clone();
        assert_eq!(layout(&cx).padding, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout(&cx).opacity, Some(0.8));

        // 1. Override padding, remove opacity (reverts to base = None), and add conditional + container rule.
        let sheet = HotStyleSheet {
            rules: vec![
                HotStyleRule {
                    classes: vec!["box".into()],
                    pseudo: 0,
                    patch: HotStylePatch::from_ops(vec![nana_ui_core::StyleOp::Padding(Some(
                        LengthSpec::Px(16.0),
                    ))]),
                    query: None,
                },
                HotStyleRule {
                    classes: vec!["on".into()],
                    pseudo: 0,
                    patch: HotStylePatch::from_ops(vec![nana_ui_core::StyleOp::Opacity(Some(0.4))]),
                    query: None,
                },
                HotStyleRule {
                    classes: vec!["box".into()],
                    pseudo: 0,
                    patch: HotStylePatch::from_ops(vec![nana_ui_core::StyleOp::Height(Some(
                        LengthSpec::Px(40.0),
                    ))]),
                    query: Some(HotSheetQuery {
                        name: None,
                        axis: HotSheetAxis::Width,
                        intervals: vec![(f32::NEG_INFINITY, 300.0)],
                    }),
                },
            ],
        };
        apply_hot_styles("HotStyleUnit", shape, sheet).unwrap();
        cx.flush_reactive().unwrap();
        assert_eq!(layout(&cx).padding, Some(LengthSpec::Px(16.0)));
        assert_eq!(
            layout(&cx).opacity,
            None,
            "removed property reverts to base"
        );

        let responsive = cx.world().responsive_rule(root).expect("hot responsive");
        assert_eq!(responsive.axis(), ResponsiveAxis::Width);
        assert_eq!(responsive.bucket_for(200.0), 0);
        assert!(responsive.variant(0).is_some());

        // 2. Toggle conditional class under hot styles.
        flag.set(true);
        cx.flush_reactive().unwrap();
        assert_eq!(layout(&cx).opacity, Some(0.4));

        // 3. Write-back of default (opacity: None or 1.0) overrides earlier rule.
        apply_hot_styles(
            "HotStyleUnit",
            shape,
            HotStyleSheet {
                rules: vec![
                    HotStyleRule {
                        classes: vec!["on".into()],
                        pseudo: 0,
                        patch: HotStylePatch::from_ops(vec![nana_ui_core::StyleOp::Opacity(Some(
                            0.2,
                        ))]),
                        query: None,
                    },
                    HotStyleRule {
                        classes: vec!["box".into(), "on".into()],
                        pseudo: 0,
                        patch: HotStylePatch::from_ops(vec![nana_ui_core::StyleOp::Opacity(Some(
                            1.0,
                        ))]),
                        query: None,
                    },
                ],
            },
        )
        .unwrap();
        cx.flush_reactive().unwrap();
        assert_eq!(layout(&cx).opacity, Some(1.0));
        assert!(cx.world().responsive_rule(root).is_none());
    }

    static LAYER1_BASE: StylePatch = StylePatch::new(|layout| {
        layout.align_items = nana_ui_core::AlignSpec::Start;
        layout.padding = Some(LengthSpec::Px(8.0));
        layout.opacity = Some(0.5);
        layout.width = Some(LengthSpec::Px(120.0));
    });
    static LAYER1_ACTIVE: StylePatch = StylePatch::new(|layout| {
        layout.padding = Some(LengthSpec::Px(14.0));
        layout.opacity = Some(0.9);
        layout.width = Some(LengthSpec::Px(240.0));
        layout.height = Some(LengthSpec::Px(48.0));
    });
    static LAYER1_NARROW: StylePatch = StylePatch::new(|layout| {
        layout.padding = Some(LengthSpec::Px(2.0));
        layout.height = Some(LengthSpec::Px(96.0));
    });
    static LAYER_RULES: [SheetRule; 3] = [
        SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &LAYER1_BASE,
            query: None,
        },
        SheetRule {
            classes: &[0, 1],
            pseudo: 0,
            patch: &LAYER1_ACTIVE,
            query: None,
        },
        SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &LAYER1_NARROW,
            query: Some(&CARD),
        },
    ];
    static LAYER_SHEET: Sheet = Sheet::new(&LAYER_RULES, &[]);
    static INLINE_PAD_SITE: StyleSite = StyleSite::new(&[(0, &INLINE_PAD_PATCH, None)]);
    static INLINE_PAD_PATCH: StylePatch = StylePatch::new(|layout| {
        layout.padding = Some(LengthSpec::Px(20.0));
    });

    #[test]
    fn four_layer_precedence_and_reactive_preservation_hold_regardless_of_order() {
        use crate::view::{column, row, signal};
        use crate::{AppContext, DocumentId};
        use nana_ui_core::{AlignSpec, PaintTransform};

        let c_base = Class::new(&LAYER_SHEET, 0);
        let c_active = Class::new(&LAYER_SHEET, 1);
        let inline_css = InlineStyle::new(Some(&INLINE_PAD_SITE), &[]);

        let active = signal(false);
        let op_sig = signal(0.3_f32);
        let tx = PaintTransform {
            e: 12.0,
            f: -4.0,
            ..PaintTransform::IDENTITY
        };
        let tx_sig = signal(Some(tx));
        let width_sig = signal(Some(LengthSpec::Px(300.0)));

        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mounted = cx
            .mount_view_root(document, move || {
                column().children((
                    // Order A: .css() before .class()
                    row().css(inline_css).class(c_base),
                    // Order B: .class() before .css()
                    row().class(c_base).css(inline_css),
                    // Static Rust builder .padding(20.0) before .class()
                    row().padding(20.0).class(c_base),
                    // Reactive Layer 3 bindings combined with dynamic Layer 1 .class_when() and Layer 2 .padding(20.0)
                    row()
                        .padding(20.0)
                        .class(c_base)
                        .class_when(c_active, active)
                        .paint_opacity(op_sig)
                        .transform(tx_sig)
                        .width(width_sig),
                ))
            })
            .unwrap();
        let children = cx
            .world()
            .node(mounted.roots()[0])
            .unwrap()
            .children
            .clone();
        let (css_first, class_first, builder_first, reactive_node) =
            (children[0], children[1], children[2], children[3]);

        // 1. Layer 2 .css() beats Layer 1 .class() in both orders, inherits non-overlapping fields,
        // and Layer 1 beats Layer 0 row() default (AlignSpec::Center -> AlignSpec::Start).
        for id in [css_first, class_first, builder_first] {
            let layout = &cx.world().node_style(id).unwrap().layout;
            assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.opacity, Some(0.5));
            assert_eq!(layout.align_items, AlignSpec::Start);
            // @container bucket (Layer 1) also respects Layer 2 padding override while keeping height!
            let rule = cx.world().responsive_rule(id).expect("responsive rule");
            let mut narrow = (**layout).clone();
            rule.variant(0).expect("narrow variant").apply(&mut narrow);
            assert_eq!(narrow.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(narrow.height, Some(LengthSpec::Px(96.0)));
        }

        // 2. Layer 3 reactive bindings beat Layer 1 & Layer 2 on mount...
        let check_reactive = |cx: &AppContext, expected_height: Option<LengthSpec>| {
            let layout = &cx.world().node_style(reactive_node).unwrap().layout;
            assert_eq!(layout.align_items, AlignSpec::Start);
            assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.opacity, Some(0.3));
            assert_eq!(layout.transforms.transform, Some(tx));
            assert_eq!(layout.width, Some(LengthSpec::Px(300.0)));
            assert_eq!(layout.height, expected_height);
        };
        check_reactive(&cx, None);

        // ...and survive toggling .class_when() on and off!
        active.set(true);
        cx.flush_reactive().unwrap();
        check_reactive(&cx, Some(LengthSpec::Px(48.0)));

        active.set(false);
        cx.flush_reactive().unwrap();
        check_reactive(&cx, None);
    }

    #[test]
    fn hot_style_patch_from_ops_applies_typed_ops_and_preserves_layers() {
        use crate::view::{__hot_register, HotStylePatch, apply_hot_styles, column, signal};
        use crate::{AppContext, DocumentId};
        use nana_ui_core::{PaintColor, StyleOp};

        static OPS_HOT_BASE: StylePatch = StylePatch::new(|layout| {
            layout.padding = Some(LengthSpec::Px(4.0));
            layout.width = Some(LengthSpec::Px(100.0));
        });
        static OPS_HOT_RULES: [SheetRule; 1] = [SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &OPS_HOT_BASE,
            query: None,
        }];
        static OPS_HOT_SHEET: Sheet =
            Sheet::with_hot(&OPS_HOT_RULES, &[], &[], "HotOpsUnit", &["box"]);

        let shape = 0x4220_0002;
        __hot_register("HotOpsUnit", shape);
        let c_box = Class::new(&OPS_HOT_SHEET, 0);
        let op_sig = signal(0.35_f32);

        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                column()
                    .padding(18.0) // Layer 2 inline override
                    .class(c_box)
                    .paint_opacity(op_sig) // Layer 3 reactive override
            })
            .unwrap()
            .roots()[0];

        let sel_bg = PaintColor::srgb([0.1, 0.4, 0.9, 0.5]);
        let sheet = HotStyleSheet {
            rules: vec![HotStyleRule {
                classes: vec!["box".into()],
                pseudo: 0,
                patch: HotStylePatch::from_ops(vec![
                    StyleOp::Padding(Some(LengthSpec::Px(2.0))),
                    StyleOp::Width(Some(LengthSpec::Px(240.0))),
                    StyleOp::Opacity(Some(0.9)),
                    StyleOp::SelectionBackground(Some(sel_bg)),
                    StyleOp::PlaceholderOpacity(Some(0.45)),
                ]),
                query: None,
            }],
        };
        apply_hot_styles("HotOpsUnit", shape, sheet).unwrap();
        cx.flush_reactive().unwrap();

        let layout = &cx.world().node_style(root).unwrap().layout;
        assert_eq!(layout.width, Some(LengthSpec::Px(240.0)));
        assert_eq!(layout.paint_colors.selection_background, Some(sel_bg));
        assert_eq!(layout.text_ext.placeholder_opacity, Some(0.45));
        assert_eq!(
            layout.padding,
            Some(LengthSpec::Px(18.0)),
            "Layer 2 inline padding must survive hot ops"
        );
        assert_eq!(
            layout.opacity,
            Some(0.35),
            "Layer 3 reactive opacity must survive hot ops"
        );
    }

    #[test]
    fn media_query_rules_react_to_viewport_theme_scale_and_accessibility_while_preserving_layers() {
        use crate::view::{column, media_env_signal, signal};
        use crate::{AppContext, DocumentId};
        use nana_ui_core::{
            MediaColorScheme, MediaCondition, MediaContrast, MediaFeature, MediaQuery,
            MediaQueryList, MediaReducedMotion, ThemeAppearance, builtin_theme_arc,
        };

        static MEDIA_BASE: StylePatch = StylePatch::new(|layout| {
            layout.width = Some(LengthSpec::Px(100.0));
            layout.height = Some(LengthSpec::Px(40.0));
            layout.padding = Some(LengthSpec::Px(4.0));
            layout.opacity = Some(1.0);
        });
        static MEDIA_WIDE: StylePatch = StylePatch::new(|layout| {
            layout.width = Some(LengthSpec::Px(480.0));
            layout.padding = Some(LengthSpec::Px(2.0));
        });
        static MEDIA_LIGHT: StylePatch = StylePatch::new(|layout| {
            layout.height = Some(LengthSpec::Px(64.0));
        });
        static MEDIA_REDUCED: StylePatch = StylePatch::new(|layout| {
            layout.margin_top = Some(LengthSpec::Px(12.0));
        });
        static MEDIA_CONTRAST: StylePatch = StylePatch::new(|layout| {
            layout.border_width = Some(3.0);
        });
        static MEDIA_HIDPI: StylePatch = StylePatch::new(|layout| {
            layout.border_radius = Some(12.0);
        });

        static MQ_WIDE: SheetMediaQuery = SheetMediaQuery::new(|| MediaQueryList {
            queries: vec![MediaQuery::from_condition(MediaCondition::feature(
                MediaFeature::min_width(720.0),
            ))],
        });
        static MQ_LIGHT: SheetMediaQuery = SheetMediaQuery::new(|| MediaQueryList {
            queries: vec![MediaQuery::from_condition(MediaCondition::feature(
                MediaFeature::PrefersColorScheme(MediaColorScheme::Light),
            ))],
        });
        static MQ_REDUCED: SheetMediaQuery = SheetMediaQuery::new(|| MediaQueryList {
            queries: vec![MediaQuery::from_condition(MediaCondition::feature(
                MediaFeature::PrefersReducedMotion(MediaReducedMotion::Reduce),
            ))],
        });
        static MQ_CONTRAST: SheetMediaQuery = SheetMediaQuery::new(|| MediaQueryList {
            queries: vec![MediaQuery::from_condition(MediaCondition::feature(
                MediaFeature::PrefersContrast(MediaContrast::More),
            ))],
        });
        static MQ_HIDPI: SheetMediaQuery = SheetMediaQuery::new(|| MediaQueryList {
            queries: vec![MediaQuery::from_condition(MediaCondition::feature(
                MediaFeature::min_resolution(2.0),
            ))],
        });

        static MEDIA_RULES: [SheetRule; 6] = [
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_BASE,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_WIDE,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_LIGHT,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_REDUCED,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_CONTRAST,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &MEDIA_HIDPI,
                query: None,
            },
        ];
        static MEDIA_GUARDS: [(usize, &'static SheetMediaQuery); 5] = [
            (1, &MQ_WIDE),
            (2, &MQ_LIGHT),
            (3, &MQ_REDUCED),
            (4, &MQ_CONTRAST),
            (5, &MQ_HIDPI),
        ];
        static MEDIA_SHEET: Sheet = Sheet::new(&MEDIA_RULES, &[]).with_media(&MEDIA_GUARDS);

        let c_card = Class::new(&MEDIA_SHEET, 0);
        let op_sig = signal(0.75_f32);
        let mut cx = AppContext::new();
        cx.set_media_viewport(600.0, 500.0);

        let document = DocumentId::new(1).unwrap();
        let env_holder = std::sync::Arc::new(std::sync::Mutex::new(None));
        let env_holder_mount = Arc::clone(&env_holder);
        let root = cx
            .mount_view_root(document, move || {
                let env_sig = media_env_signal();
                *env_holder_mount.lock().unwrap() = Some(env_sig);
                column()
                    .padding(20.0) // Layer 2 inline override
                    .class(c_card)
                    .paint_opacity(op_sig) // Layer 3 reactive override
            })
            .unwrap()
            .roots()[0];

        let env_sig = env_holder.lock().unwrap().expect("media_env_signal");
        assert_eq!(env_sig.get_untracked().width, 600.0);

        // Initial mount reflects narrow dark 1x viewport immediately.
        {
            let layout = &cx.world().node_style(root).unwrap().layout;
            assert_eq!(layout.width, Some(LengthSpec::Px(100.0)));
            assert_eq!(layout.height, Some(LengthSpec::Px(40.0)));
            assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.opacity, Some(0.75));
            assert_eq!(layout.margin_top, None);
            assert_eq!(layout.border_width, None);
            assert_eq!(layout.border_radius, None);
        }

        // Cross viewport breakpoint -> @media (min-width: 720px) activates; Layer 2/3 stay intact.
        assert!(cx.set_media_viewport(800.0, 600.0));
        cx.flush_reactive().unwrap();
        assert_eq!(env_sig.get_untracked().width, 800.0);
        {
            let layout = &cx.world().node_style(root).unwrap().layout;
            assert_eq!(layout.width, Some(LengthSpec::Px(480.0)));
            assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.opacity, Some(0.75));
        }

        // Resize within same bucket (800 -> 820) must hit StyleSite cache without extra recomposition.
        let (site, _) = MEDIA_SHEET.resolve(&[0], &[]);
        let before_compositions = site.compositions();
        assert!(cx.set_media_viewport(820.0, 600.0));
        cx.flush_reactive().unwrap();
        assert_eq!(site.compositions(), before_compositions);

        // Theme light polarity, reduced motion, high contrast, and 2x resolution.
        cx.set_theme_tokens(builtin_theme_arc(ThemeAppearance::Light))
            .unwrap();
        cx.set_reduced_motion(true);
        assert!(cx.set_high_contrast(true));
        assert!(cx.set_scale_factor(2.0));
        cx.flush_reactive().unwrap();
        {
            let layout = &cx.world().node_style(root).unwrap().layout;
            assert_eq!(layout.height, Some(LengthSpec::Px(64.0)));
            assert_eq!(layout.margin_top, Some(LengthSpec::Px(12.0)));
            assert_eq!(layout.border_width, Some(3.0));
            assert_eq!(layout.border_radius, Some(12.0));
            assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.opacity, Some(0.75));
        }
    }

    #[test]
    fn ancestor_interactive_selector_updates_descendant_layer1_while_preserving_layer2_and_layer3()
    {
        use crate::view::{column, interactive_state, row, signal};
        use crate::{AppContext, DocumentId, HeadlessInput, LayoutViewport};
        use nana_ui_input::PointerPhase;

        static CARD_RESTING: StylePatch = StylePatch::new(|layout| {
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(80.0));
            layout.border_width = Some(1.0);
            layout.paint_colors.background =
                Some(nana_ui_core::PaintColor::srgb([0.1, 0.1, 0.1, 1.0]));
        });
        static CARD_HOVER: StylePatch = StylePatch::new(|layout| {
            layout.border_width = Some(2.0);
        });
        static ICON_RESTING: StylePatch = StylePatch::new(|layout| {
            layout.width = Some(LengthSpec::Px(24.0));
            layout.height = Some(LengthSpec::Px(24.0));
            layout.padding = Some(LengthSpec::Px(2.0));
            layout.opacity = Some(0.5);
            layout.paint_colors.background =
                Some(nana_ui_core::PaintColor::srgb([0.2, 0.2, 0.2, 1.0]));
        });
        static ICON_ANC_HOVER: StylePatch = StylePatch::new(|layout| {
            layout.width = Some(LengthSpec::Px(32.0));
            layout.padding = Some(LengthSpec::Px(1.0));
            layout.opacity = Some(0.9);
        });
        static ICON_ANC_ACTIVE: StylePatch = StylePatch::new(|layout| {
            layout.height = Some(LengthSpec::Px(36.0));
        });
        static ANC_RULES: [SheetRule; 5] = [
            SheetRule {
                classes: &[0],
                pseudo: 0,
                patch: &CARD_RESTING,
                query: None,
            },
            SheetRule {
                classes: &[0],
                pseudo: PSEUDO_HOVER,
                patch: &CARD_HOVER,
                query: None,
            },
            SheetRule {
                classes: &[1],
                pseudo: 0,
                patch: &ICON_RESTING,
                query: None,
            },
            SheetRule {
                classes: &[1, 2],
                pseudo: 0,
                patch: &ICON_ANC_HOVER,
                query: None,
            },
            SheetRule {
                classes: &[1, 3],
                pseudo: 0,
                patch: &ICON_ANC_ACTIVE,
                query: None,
            },
        ];
        static ANC_SHEET: Sheet = Sheet::new(&ANC_RULES, &[]);

        let c_card = Class::new(&ANC_SHEET, 0);
        let c_icon = Class::new(&ANC_SHEET, 1);
        let c_anc_hover = Class::new(&ANC_SHEET, 2);
        let c_anc_active = Class::new(&ANC_SHEET, 3);
        let op_sig = signal(0.6_f32);

        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                let card_state = interactive_state();
                column()
                    .class(c_card)
                    .track_interactive_state(card_state, PSEUDO_HOVER | PSEUDO_ACTIVE)
                    .children((row()
                        .padding(10.0) // Layer 2 inline override
                        .class(c_icon)
                        .ancestor_pseudo(c_anc_hover, card_state, PSEUDO_HOVER)
                        .ancestor_pseudo(c_anc_active, card_state, PSEUDO_ACTIVE)
                        .paint_opacity(op_sig),))
            })
            .unwrap()
            .roots()[0];
        let icon_id = cx.world().node(root).unwrap().children[0];

        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.layout_document(document, LayoutViewport::new(400.0, 300.0))
            .unwrap();
        cx.rebuild_hit_test(document);
        let mut input = HeadlessInput::bind(&mut cx, document);

        // Resting state
        {
            let card_layout = &cx.world().node_style(root).unwrap().layout;
            let icon_layout = &cx.world().node_style(icon_id).unwrap().layout;
            assert_eq!(card_layout.border_width, Some(1.0));
            assert_eq!(icon_layout.width, Some(LengthSpec::Px(24.0)));
            assert_eq!(icon_layout.height, Some(LengthSpec::Px(24.0)));
            assert_eq!(icon_layout.padding, Some(LengthSpec::Px(10.0)));
            assert_eq!(icon_layout.opacity, Some(0.6));
        }

        // Hover inside the card (at (10, 10), which is also inside the descendant icon)
        input
            .pointer(&mut cx, PointerPhase::Move, 10.0, 10.0)
            .unwrap();
        {
            let card_layout = &cx.world().node_style(root).unwrap().layout;
            let icon_layout = &cx.world().node_style(icon_id).unwrap().layout;
            assert_eq!(card_layout.border_width, Some(2.0), "ancestor own :hover");
            assert_eq!(
                icon_layout.width,
                Some(LengthSpec::Px(32.0)),
                ".card:hover .icon activates"
            );
            assert_eq!(icon_layout.height, Some(LengthSpec::Px(24.0)));
            assert_eq!(
                icon_layout.padding,
                Some(LengthSpec::Px(10.0)),
                "Layer 2 preserved"
            );
            assert_eq!(icon_layout.opacity, Some(0.6), "Layer 3 preserved");
        }

        // Press inside the card -> .card:active .icon activates too
        input
            .pointer(&mut cx, PointerPhase::Down, 10.0, 10.0)
            .unwrap();
        {
            let icon_layout = &cx.world().node_style(icon_id).unwrap().layout;
            assert_eq!(icon_layout.width, Some(LengthSpec::Px(32.0)));
            assert_eq!(
                icon_layout.height,
                Some(LengthSpec::Px(36.0)),
                ".card:active .icon activates"
            );
            assert_eq!(icon_layout.padding, Some(LengthSpec::Px(10.0)));
            assert_eq!(icon_layout.opacity, Some(0.6));
        }

        // Release and leave -> reverts to resting
        input
            .pointer(&mut cx, PointerPhase::Up, 350.0, 250.0)
            .unwrap();
        input
            .pointer(&mut cx, PointerPhase::Move, 350.0, 250.0)
            .unwrap();
        {
            let icon_layout = &cx.world().node_style(icon_id).unwrap().layout;
            assert_eq!(icon_layout.width, Some(LengthSpec::Px(24.0)));
            assert_eq!(icon_layout.height, Some(LengthSpec::Px(24.0)));
        }
    }

    #[test]
    fn sheet_font_faces_auto_register_and_deduplicate_on_world() {
        use crate::view::{column, row};
        use crate::{AppContext, DocumentId};
        use nana_ui_core::{FontFaceRule, FontFaceSource, FontStyleSpec};

        fn build_fonts() -> Vec<FontFaceRule> {
            let mut italic = FontFaceRule::new(
                "DisplayFace",
                vec![FontFaceSource::url("nana://res/fonts/display-italic.ttf")],
            );
            italic.weight = Some(400);
            italic.style = Some(FontStyleSpec::Italic);
            vec![
                FontFaceRule::new(
                    "DisplayFace",
                    vec![FontFaceSource::url("nana://res/fonts/display.ttf")],
                ),
                // Duplicate of the first rule: must be deduplicated.
                FontFaceRule::new(
                    "DisplayFace",
                    vec![FontFaceSource::url("nana://res/fonts/display.ttf")],
                ),
                // Distinct descriptor (italic): kept.
                italic,
                // Invalid (empty family): ignored.
                FontFaceRule::new("", vec![FontFaceSource::url("nana://res/fonts/bad.ttf")]),
            ]
        }

        static FONT_RULES: [SheetRule; 1] = [SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &PAD,
            query: None,
        }];
        static FONT_SHEET: Sheet = Sheet::new(&FONT_RULES, &[]).with_font_faces(build_fonts);
        static ATTACHED_SHEET: Sheet = Sheet::new(&[], &[]).with_font_faces(|| {
            vec![FontFaceRule::new(
                "MonoFace",
                vec![FontFaceSource::url("nana://res/fonts/mono.ttf")],
            )]
        });

        let c_box = Class::new(&FONT_SHEET, 0);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                column()
                    .sheet(&ATTACHED_SHEET)
                    .children((row().class(c_box), row().class(c_box)))
            })
            .unwrap()
            .roots()[0];

        assert_eq!(cx.world().registered_font_faces().len(), 3);
        let pending = cx.take_pending_font_faces();
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0].family, "MonoFace");
        assert_eq!(pending[1].family, "DisplayFace");
        assert_eq!(pending[1].style, None);
        assert_eq!(pending[2].family, "DisplayFace");
        assert_eq!(pending[2].style, Some(FontStyleSpec::Italic));

        // Already drained; mounting more nodes from the same sheet queues nothing new.
        assert!(cx.take_pending_font_faces().is_empty());
        cx.mount_view(root, move || row().class(c_box)).unwrap();
        assert!(cx.take_pending_font_faces().is_empty());
        assert_eq!(cx.register_font_faces(&pending), 0);
    }

    #[test]
    fn placeholder_and_selection_styles_apply_to_text_input_and_text_area() {
        use crate::view::{column, text_area, text_input};
        use crate::{
            AppContext, DocumentId, LayoutViewport, MeasureTextShaper, MutationQueue, TextSelection,
        };
        use nana_ui_core::PaintColor;

        static INPUT_PSEUDO_PATCH: StylePatch = StylePatch::new(|layout| {
            layout.text_ext.placeholder_color = Some([0.8, 0.2, 0.1, 1.0]);
            layout.text_ext.placeholder_opacity = Some(0.5);
            layout.paint_colors.selection_background = Some(PaintColor::srgb([0.1, 0.6, 0.2, 0.4]));
            layout.paint_colors.selection_color = Some(PaintColor::srgb([1.0, 1.0, 0.0, 1.0]));
        });
        static INPUT_RULES: [SheetRule; 1] = [SheetRule {
            classes: &[0],
            pseudo: 0,
            patch: &INPUT_PSEUDO_PATCH,
            query: None,
        }];
        static INPUT_SHEET: Sheet = Sheet::new(&INPUT_RULES, &[]);
        let c_styled = Class::new(&INPUT_SHEET, 0);

        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .mount_view_root(document, move || {
                column().children((
                    text_input().placeholder("Type here").class(c_styled),
                    text_input().placeholder("Default placeholder"),
                    text_area().value("Hello selected world").class(c_styled),
                    text_area().value("Default selection"),
                ))
            })
            .unwrap()
            .roots()[0];
        let children = cx.world().node(root).unwrap().children.to_vec();
        let (styled_input, default_input, styled_area, default_area) =
            (children[0], children[1], children[2], children[3]);

        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, &mut MeasureTextShaper).unwrap();
        cx.layout_document(document, LayoutViewport::new(480.0, 320.0))
            .unwrap();
        cx.shape_text_for_layout(document, &mut MeasureTextShaper)
            .unwrap();

        // Select "selected" (bytes 6..14) in styled_area and "Default" (0..7) in default_area.
        let mut queue = MutationQueue::new();
        queue.set_text_selection(
            styled_area,
            TextSelection {
                anchor: 6,
                focus: 14,
                affinity: crate::TextAffinity::Downstream,
            },
        );
        queue.set_text_selection(
            default_area,
            TextSelection {
                anchor: 0,
                focus: 7,
                affinity: crate::TextAffinity::Downstream,
            },
        );
        cx.commit_mutations(queue).unwrap();
        let work = cx.take_system_work();
        cx.shape_text(&work.text, &mut MeasureTextShaper).unwrap();

        // 1. Placeholder color * placeholder opacity applied; default uses theme fallback.
        let styled_input_geom = cx
            .world()
            .text_input_geometry(styled_input)
            .expect("styled input geometry");
        let default_input_geom = cx
            .world()
            .text_input_geometry(default_input)
            .expect("default input geometry");
        let styled_ph = styled_input_geom.text.color.expect("placeholder color");
        assert!((styled_ph[0] - 0.8).abs() < 1e-4);
        assert!((styled_ph[1] - 0.2).abs() < 1e-4);
        assert!((styled_ph[2] - 0.1).abs() < 1e-4);
        assert!((styled_ph[3] - 0.5).abs() < 1e-4);
        assert_ne!(default_input_geom.text.color, Some(styled_ph));

        // 2. Selection background applied on TextArea; default uses theme fallback.
        let styled_area_geom = cx
            .world()
            .text_input_geometry(styled_area)
            .expect("styled area geometry");
        let default_area_geom = cx
            .world()
            .text_input_geometry(default_area)
            .expect("default area geometry");
        assert_eq!(styled_area_geom.selection_color, [0.1, 0.6, 0.2, 0.4]);
        assert_ne!(
            default_area_geom.selection_color,
            styled_area_geom.selection_color
        );

        // 3. Selection foreground color emitted as ExtractedTextSpan over bytes 6..14;
        //    default_area emits no selection foreground override span.
        let extracted = cx.world().extract_nodes(&[styled_area, default_area]);
        let styled_extracted = extracted
            .iter()
            .find(|n| n.id == styled_area)
            .expect("styled extracted");
        let default_extracted = extracted
            .iter()
            .find(|n| n.id == default_area)
            .expect("default extracted");
        assert!(
            styled_extracted.text_spans.iter().any(|span| {
                span.start == 6
                    && span.end == 14
                    && span.color == [1.0, 1.0, 0.0, 1.0]
                    && span.paint_color == Some(PaintColor::srgb([1.0, 1.0, 0.0, 1.0]))
            }),
            "expected selection foreground span on styled TextArea, got {:?}",
            styled_extracted.text_spans
        );
        assert!(
            default_extracted.text_spans.is_empty(),
            "unstyled TextArea should keep default text color with no selection span"
        );
    }

    #[test]
    fn patch_module_reexports_all_wave1_core_types() {
        use super::__patch::{
            FontFaceRule, FontFaceSource, FontStyleSpec, MediaColorScheme, MediaComparison,
            MediaCondition, MediaContrast, MediaEnv, MediaFeature, MediaHover, MediaOrientation,
            MediaPointer, MediaQuery, MediaQueryList, MediaReducedMotion, MediaType, StyleOp,
        };

        let _env = MediaEnv {
            media_type: MediaType::Screen,
            color_scheme: MediaColorScheme::Dark,
            reduced_motion: MediaReducedMotion::Reduce,
            contrast: MediaContrast::More,
            hover: MediaHover::Hover,
            pointer: MediaPointer::Fine,
            ..MediaEnv::default()
        };
        let _query = MediaQueryList {
            queries: vec![MediaQuery::with_type(
                false,
                MediaType::Screen,
                Some(MediaCondition::and(vec![
                    MediaCondition::feature(MediaFeature::Width(MediaComparison::Ge, 600.0)),
                    MediaCondition::feature(MediaFeature::Orientation(MediaOrientation::Landscape)),
                ])),
            )],
        };
        let _font = FontFaceRule {
            family: "Reexported".into(),
            sources: vec![FontFaceSource::local("System")],
            style: Some(FontStyleSpec::Normal),
            ..FontFaceRule::default()
        };
        let _op = StyleOp::Hidden(true);
    }
}
