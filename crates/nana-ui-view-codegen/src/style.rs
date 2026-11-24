//! A view's styles, compiled with the CSS engine while the application
//! builds: the `.vue` compiler's `<style>`, `view!`'s `<style>`, and
//! `css!`.
//!
//! Rules whose selector is a class, a compound of classes (`.card`,
//! `.card.active`), an optional subject interactive pseudo-class
//! (`:hover`, `:focus`, `:focus-visible`, `:active`), or—within `<style>`—a
//! static child (`>`) / descendant (` `) combinator of class compounds are
//! matched against each element's `class="…"` and `class:name="condition"`
//! here, in cascade order (`!important`, specificity, source order). Each
//! rule becomes a patch: the Style Model fields its declarations write and
//! their values, as `nana-ui-css` reports them ([`WrittenLayout`]),
//! including a field written with the value the default layout has.
//! Theme tokens (`var(--nana-*)`, semantic color names, `theme(…)`,
//! `light-dark(…)`, `:root[data-theme="dark"]`, `color-mix(…)`) lower into
//! theme-aware `PaintColor` values resolved at runtime against the live
//! `StyleModelRef` and `ThemeScope`. `transition` (including `transition-delay`,
//! conditional classes, and interactive pseudo states) becomes the element's
//! implicit animations, and `@keyframes` + `animation` lower to `#87
//! MotionTrack` definitions.
//!
//! Class rules inside an `@container` block compile the same way and carry
//! the block's query as data: the container it reads, the axis, and the
//! extents where it holds. The running view composes them per bucket of the
//! container's size into a responsive rule; it measures nothing itself.
//!
//! Everything else in the sheet (other selectors, ancestor interactive pseudos,
//! sibling combinators, combinators in `stylesheet!`, `@media`, `@font-face`,
//! an `@container` query the engine does not evaluate, a `transition` or
//! `animation` inside `@container`, a declaration the Style Model has no field
//! for, and what the engine skips while parsing) is a warning, not silently
//! dropped. Each warning points at what it is about: a range of the sheet's
//! text, or the element or `class` attribute in the template.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::{Attr, AttrName, AttrValue, Element, Node};
use nana_ui_core::{
    AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState, ColorFilter,
    Easing, LengthSpec, MotionCurve, PaintColor, PaintTransform, StepJump,
};
use nana_ui_css::css_motion::{
    ParsedAnimation, css_list_at, easing_from_css_keyword, first_timing_token,
    motion_curve_from_css_token, parse_animation_declarations, parse_css_time_token,
    parse_transition_shorthand, split_css_comma_list,
};
use nana_ui_css::{
    Combinator, CompoundSelector, ContainerAxis, ContainerPlanUnsupported, ContainerQuery,
    ContainerQueryUnsupported, CssVarScope, DeclarationEntry, GridTrackListParse,
    GridTrackListUnsupported, InteractivePseudo, InteractiveSelector, InteractiveStyleRule,
    KeyframeSelector, KeyframesRule, MAX_CONTAINER_BREAKPOINTS, MotionDeclarations,
    ParsedStylesheet, Selector, Specificity, StyleRule, WrittenLayout, emit_color_filter_rust,
    emit_paint_color_rust, parse_container_prelude, parse_grid_track_list_result,
    parse_stylesheet_full, plan_container_queries,
};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Lit};

const PSEUDO_HOVER: u8 = 1 << 0;
const PSEUDO_FOCUS: u8 = 1 << 1;
const PSEUDO_FOCUS_VISIBLE: u8 = 1 << 2;
const PSEUDO_ACTIVE: u8 = 1 << 3;
const MAX_CONDITIONAL_CLASSES: usize = 60;

fn pseudo_bit(pseudo: InteractivePseudo) -> u8 {
    match pseudo {
        InteractivePseudo::Hover => PSEUDO_HOVER,
        InteractivePseudo::Focus => PSEUDO_FOCUS,
        InteractivePseudo::FocusVisible => PSEUDO_FOCUS_VISIBLE,
        InteractivePseudo::Active => PSEUDO_ACTIVE,
    }
}

/// Where a rule stands in the cascade: later wins.
type Rank = (bool, (u16, u16, u16), u32);

struct Rule {
    classes: Vec<String>,
    pseudo: u8,
    rank: Rank,
    /// Rust setter statements using the local `__nana_patch` module alias.
    patch: String,
    /// The `@container` query guarding the rule, in [`Sheet::queries`].
    query: Option<usize>,
}

#[derive(Clone, Debug)]
struct TransitionItem {
    /// Names the `AnimatableProperty` variant.
    property: &'static str,
    duration_ms: f32,
    delay_ms: f32,
    easing: Easing,
}

struct TransitionRule {
    classes: Vec<String>,
    pseudo: u8,
    rank: Rank,
    items: Vec<TransitionItem>,
}

#[derive(Clone)]
enum CompiledKfValue {
    Scalar(f32),
    Color(PaintColor),
    Transform(PaintTransform),
    Filter(ColorFilter),
}

#[derive(Clone)]
struct CompiledKfStop {
    offset: f32,
    value: CompiledKfValue,
    easing: Option<Easing>,
}

#[derive(Clone)]
struct CompiledAnimItem {
    name: String,
    property: &'static str,
    stops: Vec<CompiledKfStop>,
    duration_ms: f32,
    delay_ms: f32,
    curve: MotionCurve,
    iterations: AnimationIteration,
    direction: AnimationDirection,
    fill_mode: AnimationFillMode,
    play_state: AnimationPlayState,
}

struct AnimationRule {
    classes: Vec<String>,
    pseudo: u8,
    rank: Rank,
    items: Vec<CompiledAnimItem>,
}

struct CombinatorTarget {
    /// Outermost -> closest-to-subject, matching `Selector::ancestors`.
    ancestors: Vec<(Combinator, Vec<String>)>,
    subject: Vec<String>,
    synth_class: String,
}

pub(crate) struct Sheet {
    rules: Vec<Rule>,
    transitions: Vec<TransitionRule>,
    animations: Vec<AnimationRule>,
    combinators: Vec<CombinatorTarget>,
    referenced_classes: BTreeSet<String>,
    /// The distinct queries of the sheet's supported `@container` blocks.
    queries: Vec<ContainerQuery>,
    pub(crate) warnings: Vec<StyleWarning>,
}

/// What a style warning is about.
#[derive(Debug, Clone)]
pub enum StyleAt {
    /// A byte range of the sheet's text: a selector, an at-rule, a
    /// declaration.
    Sheet(Range<usize>),
    /// An element of the template, or its `class` attribute.
    Template(Span),
}

#[derive(Debug, Clone)]
pub struct StyleWarning {
    pub at: StyleAt,
    pub message: String,
}

fn class_only_compound(compound: &CompoundSelector, allow_interactive: bool) -> Option<&[String]> {
    let classes_only = CompoundSelector {
        classes: compound.classes.clone(),
        interactive: if allow_interactive {
            compound.interactive
        } else {
            None
        },
        ..CompoundSelector::default()
    };
    (!compound.classes.is_empty() && *compound == classes_only)
        .then_some(compound.classes.as_slice())
}

/// The classes of a class-only selector; `None` for any other.
fn class_selector(selector: &Selector) -> Option<&[String]> {
    if !selector.ancestors.is_empty() {
        return None;
    }
    class_only_compound(&selector.subject, false)
}

fn static_combinator_selector(
    selector: &Selector,
) -> Option<(Vec<(Combinator, Vec<String>)>, Vec<String>)> {
    if selector.ancestors.is_empty() {
        return None;
    }
    let subject = class_only_compound(&selector.subject, false)?.to_vec();
    let mut ancestors = Vec::with_capacity(selector.ancestors.len());
    for (comb, compound) in &selector.ancestors {
        if !matches!(comb, Combinator::Child | Combinator::Descendant) {
            return None;
        }
        ancestors.push((*comb, class_only_compound(compound, false)?.to_vec()));
    }
    Some((ancestors, subject))
}

enum SelectorTarget {
    Plain(Vec<String>, u8),
    Combinator(Vec<(Combinator, Vec<String>)>, Vec<String>, u8),
}

fn interactive_selector_target(
    selector: &InteractiveSelector,
    allow_combinators: bool,
) -> Option<SelectorTarget> {
    if selector.interactive_at != selector.ancestors.len() {
        return None;
    }
    let subject = class_only_compound(&selector.subject, true)?.to_vec();
    let pseudo = pseudo_bit(selector.pseudo);
    if selector.ancestors.is_empty() {
        return Some(SelectorTarget::Plain(subject, pseudo));
    }
    if !allow_combinators {
        return None;
    }
    let mut ancestors = Vec::with_capacity(selector.ancestors.len());
    for (comb, compound) in &selector.ancestors {
        if !matches!(comb, Combinator::Child | Combinator::Descendant) {
            return None;
        }
        ancestors.push((*comb, class_only_compound(compound, false)?.to_vec()));
    }
    Some(SelectorTarget::Combinator(ancestors, subject, pseudo))
}

fn normalize_class_list(mut classes: Vec<String>) -> Vec<String> {
    classes.sort();
    classes.dedup();
    classes
}

fn intern_combinator(
    combinators: &mut Vec<CombinatorTarget>,
    referenced: &mut BTreeSet<String>,
    ancestors: Vec<(Combinator, Vec<String>)>,
    subject: Vec<String>,
) -> String {
    let ancestors: Vec<(Combinator, Vec<String>)> = ancestors
        .into_iter()
        .map(|(comb, list)| {
            for c in &list {
                referenced.insert(c.clone());
            }
            (comb, normalize_class_list(list))
        })
        .collect();
    for c in &subject {
        referenced.insert(c.clone());
    }
    let subject = normalize_class_list(subject);
    if let Some(existing) = combinators
        .iter()
        .find(|target| target.ancestors == ancestors && target.subject == subject)
    {
        return existing.synth_class.clone();
    }
    let synth_class = format!("__nana_comb_{}", combinators.len());
    combinators.push(CombinatorTarget {
        ancestors,
        subject,
        synth_class: synth_class.clone(),
    });
    synth_class
}

fn rank(important: bool, specificity: Specificity, order: u32) -> Rank {
    (
        important,
        (
            specificity.ids,
            specificity.classes_attrs,
            specificity.types,
        ),
        order,
    )
}

/// The CSS properties an implicit animation can follow, as
/// `AnimatableProperty` variants.
fn animatable(property: &str) -> &'static [&'static str] {
    match property.trim().to_ascii_lowercase().as_str() {
        "all" => &[
            "Opacity",
            "Transform",
            "Width",
            "Height",
            "Background",
            "Color",
        ],
        "opacity" => &["Opacity"],
        "transform" => &["Transform"],
        "width" => &["Width"],
        "height" => &["Height"],
        "background" | "background-color" => &["Background"],
        "color" => &["Color"],
        "filter" => &["Filter"],
        _ => &[],
    }
}

fn transition_items(
    motion: &MotionDeclarations,
    warnings: &mut Vec<String>,
) -> Vec<TransitionItem> {
    let shorthand = motion
        .transition
        .as_deref()
        .and_then(parse_transition_shorthand);
    let pick = |longhand: &Option<String>, short: Option<&String>| {
        longhand
            .clone()
            .or_else(|| short.cloned())
            .unwrap_or_default()
    };
    let mut properties = pick(
        &motion.transition_property,
        shorthand.as_ref().map(|s| &s.property),
    );
    let durations_raw = pick(
        &motion.transition_duration,
        shorthand.as_ref().map(|s| &s.duration),
    );
    let delays_raw = pick(
        &motion.transition_delay,
        shorthand.as_ref().map(|s| &s.delay),
    );
    if properties.trim().is_empty()
        && (motion.transition_duration.is_some() || motion.transition_delay.is_some())
    {
        properties = "all".into();
    }
    let durations = split_css_comma_list(&durations_raw);
    let delays = split_css_comma_list(&delays_raw);
    let timings = split_css_comma_list(&pick(
        &motion.transition_timing_function,
        shorthand.as_ref().map(|s| &s.timing_function),
    ));
    let mut items = Vec::new();
    for (index, property) in split_css_comma_list(&properties).iter().enumerate() {
        if property.trim().eq_ignore_ascii_case("none") {
            continue;
        }
        let duration_ms = if durations.is_empty() {
            0.0
        } else {
            parse_css_time_token(css_list_at(&durations, index)).unwrap_or(0.0)
        };
        let delay_ms = if delays.is_empty() {
            0.0
        } else {
            parse_css_time_token(css_list_at(&delays, index)).unwrap_or(0.0)
        };
        if duration_ms <= 0.0 && delay_ms <= 0.0 {
            continue;
        }
        let easing = if timings.is_empty() {
            easing_from_css_keyword("ease")
        } else {
            easing_from_css_keyword(
                &first_timing_token(css_list_at(&timings, index)).to_ascii_lowercase(),
            )
        };
        let variants = animatable(property);
        if variants.is_empty() {
            warnings.push(format!(
                "`transition: {property}` is not animated: only opacity, transform, filter, \
                 width, height, background and color are"
            ));
        }
        items.extend(variants.iter().map(|variant| TransitionItem {
            property: variant,
            duration_ms,
            delay_ms,
            easing,
        }));
    }
    items
}

fn sheet_vars_dual(sheet: &ParsedStylesheet) -> (CssVarScope, CssVarScope) {
    (
        sheet.custom_properties_for("light"),
        sheet.custom_properties_for("dark"),
    )
}

fn compile_dual_written(
    entries: &[&DeclarationEntry],
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
) -> WrittenLayout {
    WrittenLayout::compile_dual(entries, vars_light, vars_dark)
}

fn patch_dual(
    entries: &[&DeclarationEntry],
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
) -> String {
    compile_dual_written(entries, vars_light, vars_dark).patch_rust_stmts("__nana_patch")
}

fn extract_keyframe_props(written: &WrittenLayout) -> Vec<(&'static str, CompiledKfValue)> {
    let mut out = Vec::new();
    let mut saw_bg = false;
    let mut saw_color = false;
    let layout = &written.layout;
    for path in &written.written {
        match path.as_str() {
            "opacity" => {
                if let Some(v) = layout.opacity {
                    out.push(("Opacity", CompiledKfValue::Scalar(v)));
                }
            }
            "transforms.transform" => {
                if layout.transforms.transform_3d.is_none()
                    && layout.transforms.unsupported_transform.is_none()
                {
                    let t = layout.transforms.transform.unwrap_or_default();
                    out.push(("Transform", CompiledKfValue::Transform(t)));
                }
            }
            "paint_colors.background" | "background" if !saw_bg => {
                saw_bg = true;
                let pc = layout
                    .paint_colors
                    .background
                    .unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 0.0]));
                out.push(("Background", CompiledKfValue::Color(pc)));
            }
            "paint_colors.color" | "color" if !saw_color => {
                if let Some(pc) = layout.paint_colors.color {
                    saw_color = true;
                    out.push(("Color", CompiledKfValue::Color(pc)));
                }
            }
            // `filter: none` is the identity, so it interpolates with a filter.
            "paint.filter" => {
                let filter = layout.paint.filter.unwrap_or_default();
                out.push(("Filter", CompiledKfValue::Filter(filter)));
            }
            "width" => {
                if let Some(LengthSpec::Px(px)) = layout.width {
                    out.push(("Width", CompiledKfValue::Scalar(px)));
                }
            }
            "height" => {
                if let Some(LengthSpec::Px(px)) = layout.height {
                    out.push(("Height", CompiledKfValue::Scalar(px)));
                }
            }
            "padding" => {
                if let Some(LengthSpec::Px(px)) = layout.padding {
                    out.push(("Padding", CompiledKfValue::Scalar(px)));
                }
            }
            "margin" => {
                if let Some(LengthSpec::Px(px)) = layout.margin {
                    out.push(("Margin", CompiledKfValue::Scalar(px)));
                }
            }
            _ => {}
        }
    }
    out
}

type CompiledKeyframesMap = BTreeMap<String, BTreeMap<&'static str, Vec<CompiledKfStop>>>;

fn compile_keyframes_map(
    keyframes: &BTreeMap<String, KeyframesRule>,
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
) -> CompiledKeyframesMap {
    let mut compiled = CompiledKeyframesMap::new();
    for (name, rule) in keyframes {
        let mut by_offset: BTreeMap<u32, (f32, Vec<&DeclarationEntry>)> = BTreeMap::new();
        for block in &rule.blocks {
            for sel in &block.selectors {
                let offset = match sel {
                    KeyframeSelector::From => 0.0,
                    KeyframeSelector::To => 1.0,
                    KeyframeSelector::Percent(pct) => (pct / 100.0).clamp(0.0, 1.0),
                };
                let key = (offset * 10_000.0).round() as u32;
                let entry = by_offset.entry(key).or_insert_with(|| (offset, Vec::new()));
                entry.1.extend(block.declaration_entries.iter());
            }
        }
        let mut tracks: BTreeMap<&'static str, Vec<CompiledKfStop>> = BTreeMap::new();
        for (offset, entries) in by_offset.into_values() {
            let mut stop_easing = None;
            let mut decls = Vec::new();
            for entry in entries {
                if entry.property.starts_with("--") {
                    continue;
                }
                if entry
                    .property
                    .eq_ignore_ascii_case("animation-timing-function")
                {
                    if let MotionCurve::Easing(easing) = motion_curve_from_css_token(&entry.value) {
                        stop_easing = Some(easing);
                    }
                    continue;
                }
                decls.push(entry);
            }
            if decls.is_empty() {
                continue;
            }
            let written = compile_dual_written(&decls, vars_light, vars_dark);
            for (property, value) in extract_keyframe_props(&written) {
                tracks.entry(property).or_default().push(CompiledKfStop {
                    offset,
                    value,
                    easing: stop_easing,
                });
            }
        }
        compiled.insert(name.clone(), tracks);
    }
    compiled
}

fn animation_items(
    motion: &MotionDeclarations,
    compiled_kf: &CompiledKeyframesMap,
    warnings: &mut Vec<String>,
) -> Vec<CompiledAnimItem> {
    let parsed = parse_animation_declarations(motion);
    if !parsed.is_empty() {
        let mut out = Vec::new();
        for ParsedAnimation {
            name,
            duration_ms,
            delay_ms,
            curve,
            iterations,
            direction,
            fill_mode,
            play_state,
        } in parsed
        {
            let Some(tracks) = compiled_kf.get(&name) else {
                warnings.push(format!("`animation`: unknown `@keyframes` `{name}`"));
                continue;
            };
            if tracks.is_empty() {
                warnings.push(format!(
                    "`animation: {name}`: `@keyframes {name}` sets no animatable property"
                ));
                continue;
            }
            for (property, stops) in tracks {
                out.push(CompiledAnimItem {
                    name: name.clone(),
                    property,
                    stops: stops.clone(),
                    duration_ms,
                    delay_ms,
                    curve,
                    iterations,
                    direction,
                    fill_mode,
                    play_state,
                });
            }
        }
        return out;
    }
    if motion.animation.is_none()
        && motion.animation_name.is_none()
        && let Some(play_raw) = motion.animation_play_state.as_deref()
    {
        let play_state = match play_raw.trim().to_ascii_lowercase().as_str() {
            "paused" => Some(AnimationPlayState::Paused),
            "running" => Some(AnimationPlayState::Running),
            _ => None,
        };
        if let Some(play_state) = play_state {
            return vec![CompiledAnimItem {
                name: String::new(),
                property: "Opacity",
                stops: Vec::new(),
                duration_ms: 0.0,
                delay_ms: 0.0,
                curve: MotionCurve::Easing(Easing::Linear),
                iterations: AnimationIteration::ONCE,
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::None,
                play_state,
            }];
        }
    }
    let is_explicit_none = motion
        .animation
        .as_deref()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("none"))
        || motion
            .animation_name
            .as_deref()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("none"));
    if is_explicit_none {
        return vec![CompiledAnimItem {
            name: "none".into(),
            property: "Opacity",
            stops: Vec::new(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            curve: MotionCurve::Easing(Easing::Linear),
            iterations: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }];
    }
    Vec::new()
}

/// The class rules among `rules` as patches, one for a rule's normal and
/// one for its `!important` declarations, guarded by `query` when they are
/// an `@container` block's.
fn compile_rules(
    rules: &[StyleRule],
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
    query: Option<usize>,
    allow_combinators: bool,
    combinators: &mut Vec<CombinatorTarget>,
    referenced: &mut BTreeSet<String>,
    out: &mut Vec<Rule>,
) {
    for rule in rules {
        let (normal, important): (Vec<&DeclarationEntry>, Vec<&DeclarationEntry>) = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .partition(|entry| !entry.important);
        for (important, entries) in [(false, normal), (true, important)] {
            if entries.is_empty() {
                continue;
            }
            let patch = patch_dual(&entries, vars_light, vars_dark);
            if patch.is_empty() {
                continue;
            }
            for selector in &rule.selectors {
                let classes = if let Some(classes) = class_selector(selector) {
                    classes.to_vec()
                } else if allow_combinators
                    && let Some((ancestors, subject)) = static_combinator_selector(selector)
                {
                    vec![intern_combinator(
                        combinators,
                        referenced,
                        ancestors,
                        subject,
                    )]
                } else {
                    continue;
                };
                out.push(Rule {
                    classes,
                    pseudo: 0,
                    rank: rank(important, selector.specificity, rule.source_order),
                    patch: patch.clone(),
                    query,
                });
            }
        }
    }
}

fn compile_interactive_rules(
    rules: &[InteractiveStyleRule],
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
    query: Option<usize>,
    allow_combinators: bool,
    combinators: &mut Vec<CombinatorTarget>,
    referenced: &mut BTreeSet<String>,
    out: &mut Vec<Rule>,
) {
    for rule in rules {
        let Some(target) = interactive_selector_target(&rule.selector, allow_combinators) else {
            continue;
        };
        let (normal, important): (Vec<&DeclarationEntry>, Vec<&DeclarationEntry>) = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .partition(|entry| !entry.important);
        for (important, entries) in [(false, normal), (true, important)] {
            if entries.is_empty() {
                continue;
            }
            let patch = patch_dual(&entries, vars_light, vars_dark);
            if patch.is_empty() {
                continue;
            }
            let (classes, pseudo) = match &target {
                SelectorTarget::Plain(classes, pseudo) => (classes.clone(), *pseudo),
                SelectorTarget::Combinator(ancestors, subject, pseudo) => (
                    vec![intern_combinator(
                        combinators,
                        referenced,
                        ancestors.clone(),
                        subject.clone(),
                    )],
                    *pseudo,
                ),
            };
            out.push(Rule {
                classes,
                pseudo,
                rank: rank(important, rule.selector.specificity, rule.source_order),
                patch,
                query,
            });
        }
    }
}

#[cfg(test)]
pub(crate) fn parse(css: &str) -> Sheet {
    parse_sheet(css, true)
}

pub(crate) fn parse_sheet(css: &str, allow_combinators: bool) -> Sheet {
    let (sheet, _) = parse_stylesheet_full(css, 0);
    let (vars_light, vars_dark) = sheet_vars_dual(&sheet);
    let compiled_kf = compile_keyframes_map(&sheet.keyframes, &vars_light, &vars_dark);
    let mut rules = Vec::new();
    let mut combinators = Vec::new();
    let mut referenced_classes = BTreeSet::new();
    compile_rules(
        &sheet.static_rules,
        &vars_light,
        &vars_dark,
        None,
        allow_combinators,
        &mut combinators,
        &mut referenced_classes,
        &mut rules,
    );
    compile_interactive_rules(
        &sheet.interactive_rules,
        &vars_light,
        &vars_dark,
        None,
        allow_combinators,
        &mut combinators,
        &mut referenced_classes,
        &mut rules,
    );
    // `@container` blocks: their rules share the sheet's source order, so
    // they rank among the others where they are written.
    let mut queries: Vec<ContainerQuery> = Vec::new();
    for block in &sheet.container_rules {
        // An unsupported query holds nowhere; `check` says why.
        let Ok(query) = &block.query else {
            continue;
        };
        let index = match queries.iter().position(|known| known == query) {
            Some(index) => index,
            None => {
                queries.push(query.clone());
                queries.len() - 1
            }
        };
        compile_rules(
            &block.sheet.static_rules,
            &vars_light,
            &vars_dark,
            Some(index),
            allow_combinators,
            &mut combinators,
            &mut referenced_classes,
            &mut rules,
        );
        compile_interactive_rules(
            &block.sheet.interactive_rules,
            &vars_light,
            &vars_dark,
            Some(index),
            allow_combinators,
            &mut combinators,
            &mut referenced_classes,
            &mut rules,
        );
    }
    let mut transitions = Vec::new();
    let mut animations = Vec::new();
    for rule in &sheet.motion_rules {
        let trans_items = transition_items(&rule.motion, &mut Vec::new());
        let anim_items = animation_items(&rule.motion, &compiled_kf, &mut Vec::new());
        if trans_items.is_empty() && anim_items.is_empty() {
            continue;
        }
        for selector in &rule.selectors {
            let classes = if let Some(classes) = class_selector(selector) {
                classes.to_vec()
            } else if allow_combinators
                && let Some((ancestors, subject)) = static_combinator_selector(selector)
            {
                vec![intern_combinator(
                    &mut combinators,
                    &mut referenced_classes,
                    ancestors,
                    subject,
                )]
            } else {
                continue;
            };
            let rule_rank = rank(false, selector.specificity, rule.source_order);
            if !trans_items.is_empty() {
                transitions.push(TransitionRule {
                    classes: classes.clone(),
                    pseudo: 0,
                    rank: rule_rank,
                    items: trans_items.clone(),
                });
            }
            if !anim_items.is_empty() {
                animations.push(AnimationRule {
                    classes,
                    pseudo: 0,
                    rank: rule_rank,
                    items: anim_items.clone(),
                });
            }
        }
    }
    for rule in &sheet.interactive_rules {
        if rule.motion.is_empty() {
            continue;
        }
        let trans_items = transition_items(&rule.motion, &mut Vec::new());
        let anim_items = animation_items(&rule.motion, &compiled_kf, &mut Vec::new());
        if trans_items.is_empty() && anim_items.is_empty() {
            continue;
        }
        let Some(target) = interactive_selector_target(&rule.selector, allow_combinators) else {
            continue;
        };
        let (classes, pseudo) = match target {
            SelectorTarget::Plain(classes, pseudo) => (classes, pseudo),
            SelectorTarget::Combinator(ancestors, subject, pseudo) => (
                vec![intern_combinator(
                    &mut combinators,
                    &mut referenced_classes,
                    ancestors,
                    subject,
                )],
                pseudo,
            ),
        };
        let rule_rank = rank(false, rule.selector.specificity, rule.source_order);
        if !trans_items.is_empty() {
            transitions.push(TransitionRule {
                classes: classes.clone(),
                pseudo,
                rank: rule_rank,
                items: trans_items,
            });
        }
        if !anim_items.is_empty() {
            animations.push(AnimationRule {
                classes,
                pseudo,
                rank: rule_rank,
                items: anim_items,
            });
        }
    }
    Sheet {
        rules,
        transitions,
        animations,
        combinators,
        referenced_classes,
        queries,
        warnings: check(
            css,
            &vars_light,
            &vars_dark,
            &compiled_kf,
            allow_combinators,
        ),
    }
}

/// One top-level block of a sheet, `prelude { body }` or `prelude;`, as
/// byte ranges of the sheet: the prelude trimmed, and for a rule each
/// `;`-separated segment of its body trimmed (empty ones included, as the
/// parser counts them).
struct Block {
    range: Range<usize>,
    prelude: Range<usize>,
    segments: Vec<Range<usize>>,
}

/// Skip a comment or a quoted string starting at `at`; the index after it,
/// or `None` when `at` starts neither.
fn skip_opaque(css: &[u8], at: usize) -> Option<usize> {
    match css[at] {
        b'/' if css.get(at + 1) == Some(&b'*') => Some(
            css[at + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(css.len(), |end| at + 2 + end + 2),
        ),
        quote @ (b'"' | b'\'') => {
            let mut index = at + 1;
            while index < css.len() && css[index] != quote {
                index += if css[index] == b'\\' { 2 } else { 1 };
            }
            Some((index + 1).min(css.len()))
        }
        _ => None,
    }
}

/// `range` without surrounding whitespace and leading comments.
fn trim(css: &[u8], mut range: Range<usize>) -> Range<usize> {
    loop {
        while range.start < range.end && css[range.start].is_ascii_whitespace() {
            range.start += 1;
        }
        if range.start + 1 < range.end && css[range.start] == b'/' && css[range.start + 1] == b'*' {
            range.start = skip_opaque(css, range.start)
                .unwrap_or(range.end)
                .min(range.end);
            continue;
        }
        break;
    }
    while range.end > range.start && css[range.end - 1].is_ascii_whitespace() {
        range.end -= 1;
    }
    range
}

fn outline(css: &str) -> Vec<Block> {
    let bytes = css.as_bytes();
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let mut depth = 0usize;
        // The prelude: up to `{` or `;` outside brackets.
        let mut stop = None;
        while at < bytes.len() {
            if let Some(next) = skip_opaque(bytes, at) {
                at = next;
                continue;
            }
            match bytes[at] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth = depth.saturating_sub(1),
                b'{' | b';' if depth == 0 => {
                    stop = Some(bytes[at]);
                    break;
                }
                _ => {}
            }
            at += 1;
        }
        let prelude = trim(bytes, start..at);
        if stop != Some(b'{') {
            at = (at + 1).min(bytes.len());
            if !prelude.is_empty() {
                blocks.push(Block {
                    range: prelude.clone(),
                    prelude,
                    segments: Vec::new(),
                });
            }
            continue;
        }
        let body = at + 1;
        at = body;
        let mut braces = 1usize;
        let mut parens = 0usize;
        let mut segments = Vec::new();
        let mut segment = body;
        while at < bytes.len() {
            if let Some(next) = skip_opaque(bytes, at) {
                at = next;
                continue;
            }
            match bytes[at] {
                b'{' => braces += 1,
                b'}' => {
                    braces -= 1;
                    if braces == 0 {
                        break;
                    }
                }
                b'(' | b'[' => parens += 1,
                b')' | b']' => parens = parens.saturating_sub(1),
                b';' if braces == 1 && parens == 0 => {
                    segments.push(trim(bytes, segment..at));
                    segment = at + 1;
                }
                _ => {}
            }
            at += 1;
        }
        let end = at.min(bytes.len());
        let last = trim(bytes, segment..end);
        if !last.is_empty() {
            segments.push(last);
        }
        if bytes.get(prelude.start) == Some(&b'@') {
            segments.clear();
        }
        at = (end + 1).min(bytes.len());
        blocks.push(Block {
            range: start..at,
            prelude,
            segments,
        });
    }
    blocks
}

/// The property a declaration segment sets, lowercased.
fn property_of(text: &str) -> String {
    text.split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Each block's warnings, found by parsing it alone and pointed at its
/// prelude or the declaration they are about.
fn check(
    css: &str,
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
    compiled_kf: &CompiledKeyframesMap,
    allow_combinators: bool,
) -> Vec<StyleWarning> {
    let mut warnings = Vec::new();
    for block in outline(css) {
        check_block(
            css,
            &block,
            vars_light,
            vars_dark,
            compiled_kf,
            false,
            allow_combinators,
            &mut warnings,
        );
    }
    warnings
}

/// What follows `@name` at the start of `prelude`, when it starts so.
fn at_rule<'a>(prelude: &'a str, name: &str) -> Option<&'a str> {
    let rest = prelude.strip_prefix('@')?;
    let after = rest.get(name.len()..)?;
    (rest[..name.len()].eq_ignore_ascii_case(name)
        && !after.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_'))
    .then_some(after)
}

/// The text between an at-rule block's braces.
fn body_of(css: &str, block: &Block) -> Range<usize> {
    let open = css[block.prelude.end..block.range.end]
        .find('{')
        .map_or(block.range.end, |at| block.prelude.end + at + 1);
    let close = if css[..block.range.end].ends_with('}') {
        block.range.end - 1
    } else {
        block.range.end
    };
    open..close.max(open)
}

/// [`outline`] of `range` of `css`, its ranges in `css`.
fn outline_within(css: &str, range: Range<usize>) -> Vec<Block> {
    let shift = |inner: Range<usize>| inner.start + range.start..inner.end + range.start;
    outline(&css[range.clone()])
        .into_iter()
        .map(|block| Block {
            range: shift(block.range),
            prelude: shift(block.prelude),
            segments: block.segments.into_iter().map(shift).collect(),
        })
        .collect()
}

/// The `,`-separated selectors of a prelude, trimmed.
fn selector_parts(css: &str, prelude: Range<usize>) -> Vec<Range<usize>> {
    let bytes = css.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut part = prelude.start;
    let mut at = prelude.start;
    while at < prelude.end {
        if let Some(next) = skip_opaque(bytes, at) {
            at = next;
            continue;
        }
        match bytes[at] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(trim(bytes, part..at));
                part = at + 1;
            }
            _ => {}
        }
        at += 1;
    }
    parts.push(trim(bytes, part..prelude.end));
    parts
}

/// What the engine skips while parsing `block`, as its report counts it:
/// each selector it does not parse, and an at-rule it skips whole
/// (`@import`, which a view never loads; an `@supports` whose condition
/// does not hold; an unknown at-rule). An `@supports` or `@layer` that
/// applies is checked inside.
fn check_skipped(css: &str, block: &Block, warnings: &mut Vec<StyleWarning>) {
    let prelude = &css[block.prelude.clone()];
    if prelude.starts_with('@') {
        if at_rule(prelude, "container").is_some() {
            return;
        }
        let in_place =
            at_rule(prelude, "supports").is_some() || at_rule(prelude, "layer").is_some();
        let has_body = block.range != block.prelude;
        // Its own prelude alone: what it holds is checked below.
        let text = if in_place && has_body {
            format!("{prelude} {{}}")
        } else {
            css[block.range.clone()].to_owned()
        };
        if parse_stylesheet_full(&text, 0).1.skipped_at_rules > 0 {
            let keyword = prelude
                .split(|c: char| c.is_whitespace() || c == '(' || c == '"' || c == '\'')
                .next()
                .unwrap_or(prelude);
            let message = if at_rule(prelude, "import").is_some() {
                format!("`@import` is not loaded for L3 views; `{prelude}` is ignored")
            } else if at_rule(prelude, "supports").is_some() {
                format!("the condition of `{prelude}` does not hold, so its rules are ignored")
            } else {
                format!("`{keyword}` is not compiled for L3 views; `{prelude}` is ignored")
            };
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(block.prelude.clone()),
                message,
            });
        } else if in_place && has_body {
            for inner in outline_within(css, body_of(css, block)) {
                check_skipped(css, &inner, warnings);
            }
        }
        return;
    }
    if parse_stylesheet_full(&css[block.range.clone()], 0)
        .1
        .skipped_selectors
        == 0
    {
        return;
    }
    // The rule with one selector at a time finds those it skips.
    let body = &css[block.prelude.end..block.range.end];
    for part in selector_parts(css, block.prelude.clone()) {
        let selector = &css[part.clone()];
        if parse_stylesheet_full(&format!("{selector}{body}"), 0)
            .1
            .skipped_selectors
            > 0
        {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(part),
                message: format!(
                    "the selector `{selector}` is not one the CSS engine parses and is ignored"
                ),
            });
        }
    }
}

fn check_keyframes_block(
    css: &str,
    block: &Block,
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
    warnings: &mut Vec<StyleWarning>,
) {
    let body = body_of(css, block);
    let inner_blocks = outline_within(css, body);
    let (_, report) = parse_stylesheet_full(&css[block.range.clone()], 0);
    if report.skipped_at_rules > 0 {
        check_skipped(css, block, warnings);
        return;
    }
    for inner in inner_blocks {
        let sel_text = &css[inner.prelude.clone()];
        let stop_body = &css[body_of(css, &inner)];
        let probe = format!("@keyframes __k {{ {sel_text} {{ {stop_body} }} }}");
        let (parsed, _) = parse_stylesheet_full(&probe, 0);
        let Some(kf) = parsed.keyframes.get("__k") else {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(inner.prelude.clone()),
                message: format!(
                    "`{sel_text}` is not a valid `@keyframes` stop selector and is ignored"
                ),
            });
            continue;
        };
        let Some(kb) = kf.blocks.first() else {
            continue;
        };
        for entry in &kb.declaration_entries {
            if entry.property.starts_with("--")
                || entry
                    .property
                    .eq_ignore_ascii_case("animation-timing-function")
            {
                continue;
            }
            let written = compile_dual_written(&[entry], vars_light, vars_dark);
            if extract_keyframe_props(&written).is_empty() {
                let prop = entry.property.to_ascii_lowercase();
                let at = inner
                    .segments
                    .get(entry.index as usize)
                    .filter(|seg| property_of(&css[(*seg).clone()]) == prop)
                    .or_else(|| {
                        inner
                            .segments
                            .iter()
                            .find(|seg| property_of(&css[(*seg).clone()]) == prop)
                    })
                    .cloned()
                    .unwrap_or_else(|| inner.prelude.clone());
                warnings.push(StyleWarning {
                    at: StyleAt::Sheet(at),
                    message: format!(
                        "`{}` in `@keyframes` is not animated: only opacity, transform, filter, \
                         width, height, padding, margin, color and background are",
                        entry.text()
                    ),
                });
            }
        }
    }
}

fn check_motion_declarations(
    css: &str,
    block: &Block,
    prelude: &str,
    motion: &MotionDeclarations,
    compiled_kf: &CompiledKeyframesMap,
    in_container: bool,
    selector_ok: bool,
    declaration: &dyn Fn(Option<u32>, &dyn Fn(&str) -> bool) -> Range<usize>,
    warnings: &mut Vec<StyleWarning>,
) {
    let has_transition = motion.transition.is_some()
        || motion.transition_property.is_some()
        || motion.transition_duration.is_some()
        || motion.transition_delay.is_some();
    let has_animation = motion.animation.is_some()
        || motion.animation_name.is_some()
        || motion.animation_duration.is_some()
        || motion.animation_delay.is_some()
        || motion.animation_iteration_count.is_some()
        || motion.animation_direction.is_some()
        || motion.animation_fill_mode.is_some()
        || motion.animation_play_state.is_some();
    let _ = css;
    if in_container {
        if has_transition {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(declaration(None, &|p| p.starts_with("transition"))),
                message: "a `transition` inside `@container` is not compiled and is ignored".into(),
            });
        }
        if has_animation {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(declaration(None, &|p| p.starts_with("animation"))),
                message: "an `animation` inside `@container` is not compiled and is ignored".into(),
            });
        }
        return;
    }
    let mut trans_valid = false;
    if has_transition {
        let mut messages = Vec::new();
        let items = transition_items(motion, &mut messages);
        let at = declaration(None, &|p| p.starts_with("transition"));
        for message in messages {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(at.clone()),
                message,
            });
        }
        trans_valid = !items.is_empty();
    }
    let mut anim_valid = false;
    if has_animation {
        let mut messages = Vec::new();
        let items = animation_items(motion, compiled_kf, &mut messages);
        let at = declaration(None, &|p| p.starts_with("animation"));
        for message in messages {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(at.clone()),
                message,
            });
        }
        anim_valid = !items.is_empty();
    }
    if !selector_ok && trans_valid {
        warnings.push(StyleWarning {
            at: StyleAt::Sheet(block.prelude.clone()),
            message: format!(
                "the `transition` for `{prelude}` is ignored: L3 views compile class selectors only"
            ),
        });
    }
    if !selector_ok && anim_valid {
        warnings.push(StyleWarning {
            at: StyleAt::Sheet(block.prelude.clone()),
            message: format!(
                "the `animation` for `{prelude}` is ignored: L3 views compile class selectors only"
            ),
        });
    }
}

/// One block's warnings; `in_container` when it is inside an `@container`
/// block, whose class rules compile and whose other content does not.
fn check_block(
    css: &str,
    block: &Block,
    vars_light: &CssVarScope,
    vars_dark: &CssVarScope,
    compiled_kf: &CompiledKeyframesMap,
    in_container: bool,
    allow_combinators: bool,
    warnings: &mut Vec<StyleWarning>,
) {
    let prelude = &css[block.prelude.clone()];
    let before = warnings.len();
    let mut warn = |range: Range<usize>, message: String| {
        warnings.push(StyleWarning {
            at: StyleAt::Sheet(range),
            message,
        })
    };
    if let Some(condition) = at_rule(prelude, "container") {
        if in_container {
            warn(
                block.prelude.clone(),
                format!(
                    "`@container` inside another `@container` is not compiled; `{prelude}` is \
                     ignored"
                ),
            );
            return;
        }
        match parse_container_prelude(condition) {
            Err(why) => warn(
                block.prelude.clone(),
                format!("`@container` with {why} is not compiled; `{prelude}` is ignored"),
            ),
            Ok(_) => {
                for inner in outline_within(css, body_of(css, block)) {
                    check_block(
                        css,
                        &inner,
                        vars_light,
                        vars_dark,
                        compiled_kf,
                        true,
                        allow_combinators,
                        warnings,
                    );
                }
            }
        }
        return;
    }
    if in_container && prelude.starts_with('@') {
        let keyword = prelude
            .split(|c: char| c.is_whitespace() || c == '(')
            .next()
            .unwrap_or(prelude);
        warn(
            block.prelude.clone(),
            format!("`{keyword}` inside `@container` is not compiled; `{prelude}` is ignored"),
        );
        return;
    }
    if at_rule(prelude, "keyframes").is_some() {
        check_keyframes_block(css, block, vars_light, vars_dark, warnings);
        return;
    }
    let (sheet, _) = parse_stylesheet_full(&css[block.range.clone()], 0);
    let declaration = |index: Option<u32>, matches: &dyn Fn(&str) -> bool| {
        index
            .and_then(|index| block.segments.get(index as usize))
            .filter(|segment| matches(&property_of(&css[(*segment).clone()])))
            .or_else(|| {
                block
                    .segments
                    .iter()
                    .find(|segment| matches(&property_of(&css[(*segment).clone()])))
            })
            .cloned()
            .unwrap_or_else(|| block.prelude.clone())
    };
    let skipped = [
        (!sheet.media_rules.is_empty(), "`@media` blocks"),
        (!sheet.font_faces.is_empty(), "`@font-face` rules"),
        (
            !sheet.generated_pseudo_rules.is_empty() || !sheet.scrollbar_pseudo_rules.is_empty(),
            "pseudo-element rules",
        ),
    ];
    for (skipped, what) in skipped {
        if skipped {
            warn(
                block.prelude.clone(),
                format!("{what} are not compiled for L3 views; `{prelude}` is ignored"),
            );
        }
    }
    for rule in &sheet.static_rules {
        let entries: Vec<&DeclarationEntry> = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let selector_ok = |s: &Selector| {
            class_selector(s).is_some()
                || (allow_combinators && static_combinator_selector(s).is_some())
        };
        if rule.selectors.iter().any(|s| !selector_ok(s)) {
            warn(
                block.prelude.clone(),
                format!(
                    "the rule for `{prelude}` is ignored: L3 views compile class selectors \
                     (`.a`, `.a.b`) only"
                ),
            );
        }
        for entry in entries {
            let property = entry.property.to_ascii_lowercase();
            if let Some(message) = unsupported_or_removed_css_warning(&property, entry) {
                warn(declaration(Some(entry.index), &|p| p == property), message);
            } else if patch_dual(&[entry], vars_light, vars_dark).is_empty() {
                warn(
                    declaration(Some(entry.index), &|p| p == property),
                    format!(
                        "`{}` sets nothing in the Style Model and is ignored",
                        entry.text()
                    ),
                );
            }
        }
    }
    for rule in &sheet.interactive_rules {
        let entries: Vec<&DeclarationEntry> = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .collect();
        let supported = interactive_selector_target(&rule.selector, allow_combinators).is_some();
        if !supported && !entries.is_empty() {
            let why = if rule.selector.interactive_at != rule.selector.ancestors.len() {
                "L3 views only compile `:hover` / `:focus` / `:focus-visible` / `:active` on the \
                 subject element"
            } else {
                "L3 views compile class selectors only"
            };
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(block.prelude.clone()),
                message: format!("the rule for `{prelude}` is ignored: {why}"),
            });
        }
        for entry in entries {
            let property = entry.property.to_ascii_lowercase();
            if let Some(message) = unsupported_or_removed_css_warning(&property, entry) {
                warnings.push(StyleWarning {
                    at: StyleAt::Sheet(declaration(Some(entry.index), &|p| p == property)),
                    message,
                });
            } else if patch_dual(&[entry], vars_light, vars_dark).is_empty() {
                warnings.push(StyleWarning {
                    at: StyleAt::Sheet(declaration(Some(entry.index), &|p| p == property)),
                    message: format!(
                        "`{}` sets nothing in the Style Model and is ignored",
                        entry.text()
                    ),
                });
            }
        }
        if !rule.motion.is_empty() {
            check_motion_declarations(
                css,
                block,
                prelude,
                &rule.motion,
                compiled_kf,
                in_container,
                supported,
                &declaration,
                warnings,
            );
        }
    }
    for rule in &sheet.motion_rules {
        let selector_ok = rule.selectors.iter().all(|s| {
            class_selector(s).is_some()
                || (allow_combinators && static_combinator_selector(s).is_some())
        });
        check_motion_declarations(
            css,
            block,
            prelude,
            &rule.motion,
            compiled_kf,
            in_container,
            selector_ok,
            &declaration,
            warnings,
        );
    }
    // What the engine skips while parsing, unless the at-rule is already
    // warned about as a whole.
    if !prelude.starts_with('@')
        || !warnings[before..]
            .iter()
            .any(|warning| matches!(&warning.at, StyleAt::Sheet(at) if *at == block.prelude))
    {
        check_skipped(css, block, warnings);
    }
    // `@container` or `@keyframes` inside another block that compiles
    // (`@supports`, `@layer`): checked as if it stood alone.
    if (!sheet.container_rules.is_empty() || !sheet.keyframes.is_empty())
        && prelude.starts_with('@')
    {
        for inner in outline_within(css, body_of(css, block)) {
            let inner_prelude = &css[inner.prelude.clone()];
            if at_rule(inner_prelude, "container").is_some()
                || at_rule(inner_prelude, "keyframes").is_some()
            {
                check_block(
                    css,
                    &inner,
                    vars_light,
                    vars_dark,
                    compiled_kf,
                    in_container,
                    allow_combinators,
                    warnings,
                );
            }
        }
    }
}

fn unsupported_or_removed_css_warning(property: &str, entry: &DeclarationEntry) -> Option<String> {
    let trimmed = entry.value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    ) || lower.contains("var(")
    {
        return None;
    }
    match property {
        "display" => match lower.as_str() {
            "block" | "flow-root" => Some(format!(
                "`{}`: legacy block display (`{lower}`) was removed; use `display: flex; \
                 flex-direction: column` (or `grid` / `layers`) instead; it is ignored",
                entry.text()
            )),
            "inline-block" => Some(format!(
                "`{}`: `inline-block` was removed; use `display: inline-flex` or \
                 `display: inline-grid` instead; it is ignored",
                entry.text()
            )),
            "contents" => Some(format!(
                "`{}`: `display: contents` is not supported in NanaUI's retained Style Model; \
                 remove the wrapper or move layout styles onto its children; it is ignored",
                entry.text()
            )),
            _ => None,
        },
        "float" | "clear" => Some(format!(
            "`{}`: CSS floats (`float` / `clear`) were removed; use `display: flex` or \
             `display: grid` instead; it is ignored",
            entry.text()
        )),
        "grid-template-columns" | "grid-template-rows" if !lower.eq("none") => {
            match parse_grid_track_list_result(trimmed, None) {
                GridTrackListParse::Tracks(_)
                | GridTrackListParse::RepeatAuto(_)
                | GridTrackListParse::Subgrid => None,
                GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid) => {
                    Some(format!(
                        "`{}`: mixing `subgrid` with explicit tracks is not supported in \
                         `{property}`; use standalone `subgrid` (with optional `[line-name]` \
                         groups); it is ignored",
                        entry.text()
                    ))
                }
                GridTrackListParse::Unsupported(GridTrackListUnsupported::NestedRepeat) => {
                    Some(format!(
                        "`{}`: nested `repeat(...)` in `{property}` is not supported; it is ignored",
                        entry.text()
                    ))
                }
                GridTrackListParse::Unsupported(kind) => Some(format!(
                    "`{}`: unsupported grid track syntax (`{kind:?}`) in `{property}`; it is \
                     ignored",
                    entry.text()
                )),
                GridTrackListParse::Invalid => Some(format!(
                    "`{}`: `{property}` could not parse this track list and is ignored",
                    entry.text()
                )),
            }
        }
        _ => None,
    }
}

/// Emit a typed [`StylePatch`] closure body from compiled setter statements
/// (`stmts`), binding the runtime crate's `view::__patch` module as
/// `__nana_patch` without interpolating `$crate` through string parsing.
fn patch_tokens(stmts: &str, runtime: &TokenStream) -> TokenStream {
    let stmts: TokenStream = stmts
        .parse()
        .expect("compiled style patch statements parse as Rust tokens");
    quote! {
        |__nana_layout: &mut #runtime::LayoutStyle| {
            #[allow(unused_imports)]
            use #runtime::view::__patch as __nana_patch;
            #stmts
        }
    }
}

fn easing_tokens(easing: Easing, runtime: &TokenStream) -> TokenStream {
    match easing {
        Easing::Linear => quote!(#runtime::Easing::Linear),
        Easing::EaseOutCubic => quote!(#runtime::Easing::EaseOutCubic),
        Easing::EaseInOutCubic => quote!(#runtime::Easing::EaseInOutCubic),
        Easing::CubicBezier([a, b, c, d]) => {
            quote!(#runtime::Easing::CubicBezier([#a, #b, #c, #d]))
        }
    }
}

/// Blocks that build no node of their own to style. The others (`Block`,
/// `Virtual`, `Transition`, `TransitionGroup`, `KeepAlive`) style the
/// container of the list or chain they hold, and `Teleport` the anchor its
/// content is built in.
const BLOCKS: &[&str] = &["Suspense", "ErrorBoundary"];

/// A sheet's rules and transitions as the runtime's `view::Sheet`: patch
/// statics, then the sheet, both in cascade order. `classes` numbers every
/// class a rule or transition names.
struct SheetItems {
    classes: Vec<String>,
    items: Vec<TokenStream>,
}

impl Sheet {
    fn class_index(classes: &mut Vec<String>, class: &str) -> u16 {
        match classes.iter().position(|known| known == class) {
            Some(at) => at as u16,
            None => {
                classes.push(class.to_owned());
                (classes.len() - 1) as u16
            }
        }
    }

    /// The statics of this sheet, `name` the sheet's.
    fn items(&self, name: &syn::Ident, runtime: &TokenStream) -> SheetItems {
        let mut classes = Vec::new();
        let mut patches: BTreeMap<&str, usize> = BTreeMap::new();
        let mut queries: BTreeSet<usize> = BTreeSet::new();
        let mut items = Vec::new();
        let mut ordered: Vec<&Rule> = self.rules.iter().collect();
        ordered.sort_by_key(|rule| rule.rank);
        let mut rules = Vec::new();
        for rule in ordered {
            let next = patches.len();
            let index = *patches.entry(&rule.patch).or_insert(next);
            let patch = format_ident!("{name}_PATCH_{index}");
            if index == next {
                let apply = patch_tokens(&rule.patch, runtime);
                items.push(quote! {
                    static #patch: #runtime::view::StylePatch =
                        #runtime::view::StylePatch::new(#apply);
                });
            }
            let query = match rule.query {
                Some(index) => {
                    let query = format_ident!("{name}_QUERY_{index}");
                    if queries.insert(index) {
                        let value = query_tokens(&self.queries[index], runtime);
                        items.push(quote! {
                            static #query: #runtime::view::SheetQuery = #value;
                        });
                    }
                    quote!(::core::option::Option::Some(&#query))
                }
                None => quote!(::core::option::Option::None),
            };
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            let pseudo = rule.pseudo;
            rules.push(quote! {
                #runtime::view::SheetRule {
                    classes: &[#(#needs),*],
                    pseudo: #pseudo,
                    patch: &#patch,
                    query: #query,
                }
            });
        }
        let mut ordered: Vec<&TransitionRule> = self.transitions.iter().collect();
        ordered.sort_by_key(|rule| rule.rank);
        let mut transitions = Vec::new();
        for rule in ordered {
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            let pseudo = rule.pseudo;
            let animate = animate_tokens(&rule.items, runtime);
            transitions.push(quote! {
                #runtime::view::SheetTransition {
                    classes: &[#(#needs),*],
                    pseudo: #pseudo,
                    animate: &#animate,
                }
            });
        }
        let mut ordered_anims: Vec<&AnimationRule> = self.animations.iter().collect();
        ordered_anims.sort_by_key(|rule| rule.rank);
        let mut animations = Vec::new();
        let mut kf_counter = 0usize;
        for rule in ordered_anims {
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            let pseudo = rule.pseudo;
            let mut anim_entries = Vec::with_capacity(rule.items.len());
            for anim in &rule.items {
                let stops_static = if anim.stops.is_empty() {
                    quote!(&[])
                } else {
                    let kf_ident = format_ident!("{name}_KF_{kf_counter}");
                    kf_counter += 1;
                    let stop_tokens = anim
                        .stops
                        .iter()
                        .map(|stop| keyframe_stop_tokens(stop, runtime));
                    let stop_len = anim.stops.len();
                    items.push(quote! {
                        static #kf_ident: [#runtime::view::CompiledKeyframe; #stop_len] =
                            [#(#stop_tokens),*];
                    });
                    quote!(&#kf_ident)
                };
                anim_entries.push(animation_item_tokens(anim, stops_static, runtime));
            }
            animations.push(quote! {
                #runtime::view::SheetAnimation {
                    classes: &[#(#needs),*],
                    pseudo: #pseudo,
                    animations: &[#(#anim_entries),*],
                }
            });
        }
        if animations.is_empty() {
            items.push(quote! {
                static #name: #runtime::view::Sheet =
                    #runtime::view::Sheet::new(&[#(#rules),*], &[#(#transitions),*]);
            });
        } else {
            items.push(quote! {
                static #name: #runtime::view::Sheet = #runtime::view::Sheet::with_animations(
                    &[#(#rules),*],
                    &[#(#transitions),*],
                    &[#(#animations),*],
                );
            });
        }
        SheetItems { classes, items }
    }
}

/// A compiled `@container` query as the runtime's `view::SheetQuery`. Each
/// bound is written as its bits: `max-width: 480px` ends at
/// `480f32.next_up()`, which a decimal literal need not round-trip, and an
/// open end is an infinity, which has no literal.
fn query_tokens(query: &ContainerQuery, runtime: &TokenStream) -> TokenStream {
    let name = match &query.name {
        Some(name) => quote!(::core::option::Option::Some(#name)),
        None => quote!(::core::option::Option::None),
    };
    let axis = match query.axis {
        ContainerAxis::Inline => quote!(#runtime::ResponsiveAxis::Inline),
        ContainerAxis::Block => quote!(#runtime::ResponsiveAxis::Block),
        ContainerAxis::Width => quote!(#runtime::ResponsiveAxis::Width),
        ContainerAxis::Height => quote!(#runtime::ResponsiveAxis::Height),
    };
    let bound = |value: f32| {
        let bits = value.to_bits();
        quote!(::core::primitive::f32::from_bits(#bits))
    };
    let intervals = query.intervals.iter().map(|interval| {
        let (lo, hi) = (bound(interval.lo), bound(interval.hi));
        quote!((#lo, #hi))
    });
    quote!(#runtime::view::SheetQuery::new(#name, #axis, &[#(#intervals),*]))
}

fn animate_tokens(items: &[TransitionItem], runtime: &TokenStream) -> TokenStream {
    let items = items.iter().map(|item| {
        let property = format_ident!("{}", item.property);
        let easing = easing_tokens(item.easing, runtime);
        let nanos = (f64::from(item.duration_ms.max(0.0)) * 1_000_000.0).round() as u64;
        let delay_nanos = (f64::from(item.delay_ms.max(0.0)) * 1_000_000.0).round() as u64;
        let delay = if delay_nanos > 0 {
            quote!(.delay(::std::time::Duration::from_nanos(#delay_nanos)))
        } else {
            quote!()
        };
        quote! {
            #runtime::view::Implicit::new(
                #runtime::AnimatableProperty::#property,
                ::std::time::Duration::from_nanos(#nanos),
            )
            #delay
            .ease(#easing)
        }
    });
    quote!([#(#items),*])
}

fn keyframe_stop_tokens(stop: &CompiledKfStop, runtime: &TokenStream) -> TokenStream {
    let offset = stop.offset;
    let value = match &stop.value {
        CompiledKfValue::Scalar(v) => quote!(#runtime::view::CompiledMotionValue::Scalar(#v)),
        CompiledKfValue::Color(pc) => {
            let expr: TokenStream = emit_paint_color_rust(pc, "__nana_patch")
                .parse()
                .expect("compiled PaintColor tokens parse as Rust expr");
            quote! {
                #runtime::view::CompiledMotionValue::Color(|| {
                    #[allow(unused_imports)]
                    use #runtime::view::__patch as __nana_patch;
                    #expr
                })
            }
        }
        CompiledKfValue::Filter(filter) => {
            let expr: TokenStream = emit_color_filter_rust(filter, "__nana_patch")
                .parse()
                .expect("compiled ColorFilter tokens parse as Rust expr");
            quote! {
                #runtime::view::CompiledMotionValue::Filter(|| {
                    #[allow(unused_imports)]
                    use #runtime::view::__patch as __nana_patch;
                    #expr
                })
            }
        }
        CompiledKfValue::Transform(t) => {
            let (a, b, c, d, e, f) = (t.a, t.b, t.c, t.d, t.e, t.f);
            quote! {
                #runtime::view::CompiledMotionValue::Transform(
                    #runtime::view::__patch::PaintTransform {
                        a: #a,
                        b: #b,
                        c: #c,
                        d: #d,
                        e: #e,
                        f: #f,
                    }
                )
            }
        }
    };
    let easing = match stop.easing {
        Some(easing) => {
            let easing = easing_tokens(easing, runtime);
            quote!(::core::option::Option::Some(#easing))
        }
        None => quote!(::core::option::Option::None),
    };
    quote! {
        #runtime::view::CompiledKeyframe {
            offset: #offset,
            value: #value,
            easing: #easing,
        }
    }
}

fn motion_curve_tokens(curve: MotionCurve, runtime: &TokenStream) -> TokenStream {
    match curve {
        MotionCurve::Easing(easing) => {
            let easing = easing_tokens(easing, runtime);
            quote!(#runtime::view::__patch::MotionCurve::Easing(#easing))
        }
        MotionCurve::Steps { count, jump } => {
            let jump = match jump {
                StepJump::Start => quote!(#runtime::view::__patch::StepJump::Start),
                StepJump::End => quote!(#runtime::view::__patch::StepJump::End),
                StepJump::None => quote!(#runtime::view::__patch::StepJump::None),
                StepJump::Both => quote!(#runtime::view::__patch::StepJump::Both),
            };
            quote!(#runtime::view::__patch::MotionCurve::Steps { count: #count, jump: #jump })
        }
        _ => {
            let easing = easing_tokens(Easing::EaseOutCubic, runtime);
            quote!(#runtime::view::__patch::MotionCurve::Easing(#easing))
        }
    }
}

fn animation_item_tokens(
    anim: &CompiledAnimItem,
    stops: TokenStream,
    runtime: &TokenStream,
) -> TokenStream {
    let name = &anim.name;
    let property = format_ident!("{}", anim.property);
    let duration_ms = anim.duration_ms;
    let delay_ms = anim.delay_ms;
    let curve = motion_curve_tokens(anim.curve, runtime);
    let iterations = match anim.iterations {
        AnimationIteration::Count(n) => {
            quote!(#runtime::view::__patch::AnimationIteration::Count(#n))
        }
        AnimationIteration::Infinite => {
            quote!(#runtime::view::__patch::AnimationIteration::Infinite)
        }
    };
    let direction = match anim.direction {
        AnimationDirection::Normal => quote!(#runtime::view::__patch::AnimationDirection::Normal),
        AnimationDirection::Reverse => quote!(#runtime::view::__patch::AnimationDirection::Reverse),
        AnimationDirection::Alternate => {
            quote!(#runtime::view::__patch::AnimationDirection::Alternate)
        }
        AnimationDirection::AlternateReverse => {
            quote!(#runtime::view::__patch::AnimationDirection::AlternateReverse)
        }
    };
    let fill_mode = match anim.fill_mode {
        AnimationFillMode::None => quote!(#runtime::view::__patch::AnimationFillMode::None),
        AnimationFillMode::Forwards => quote!(#runtime::view::__patch::AnimationFillMode::Forwards),
        AnimationFillMode::Backwards => {
            quote!(#runtime::view::__patch::AnimationFillMode::Backwards)
        }
        AnimationFillMode::Both => quote!(#runtime::view::__patch::AnimationFillMode::Both),
    };
    let play_state = match anim.play_state {
        AnimationPlayState::Running => quote!(#runtime::view::__patch::AnimationPlayState::Running),
        AnimationPlayState::Paused => quote!(#runtime::view::__patch::AnimationPlayState::Paused),
    };
    quote! {
        #runtime::view::KeyframeAnimation {
            name: #name,
            property: #runtime::AnimatableProperty::#property,
            keyframes: #stops,
            duration_ms: #duration_ms,
            delay_ms: #delay_ms,
            curve: #curve,
            iterations: #iterations,
            direction: #direction,
            fill_mode: #fill_mode,
            play_state: #play_state,
        }
    }
}

#[derive(Default)]
struct AncestorFrame {
    fixed: Vec<String>,
    conditional: Vec<(String, Expr)>,
}

fn match_compound(
    required: &[String],
    fixed: &[String],
    conditional: &[(String, Expr)],
) -> Option<Vec<Expr>> {
    let mut exprs = Vec::new();
    for class in required {
        if fixed.iter().any(|known| known == class) {
            continue;
        }
        if let Some((_, expr)) = conditional.iter().find(|(known, _)| known == class) {
            exprs.push(expr.clone());
            continue;
        }
        return None;
    }
    Some(exprs)
}

fn match_chain(chain: &[(Combinator, Vec<String>)], ancestors: &[AncestorFrame]) -> Vec<Vec<Expr>> {
    if chain.is_empty() {
        return vec![Vec::new()];
    }
    let ((comb, required), rest_chain) = (&chain[chain.len() - 1], &chain[..chain.len() - 1]);
    match comb {
        Combinator::Child => {
            let Some((parent, rest_ancestors)) = ancestors.split_last() else {
                return Vec::new();
            };
            let Some(parent_conds) = match_compound(required, &parent.fixed, &parent.conditional)
            else {
                return Vec::new();
            };
            match_chain(rest_chain, rest_ancestors)
                .into_iter()
                .map(|mut clause| {
                    clause.extend(parent_conds.iter().cloned());
                    clause
                })
                .collect()
        }
        Combinator::Descendant => {
            let mut clauses = Vec::new();
            for idx in (1..=ancestors.len()).rev() {
                let candidate = &ancestors[idx - 1];
                let rest_ancestors = &ancestors[..idx - 1];
                let Some(anc_conds) =
                    match_compound(required, &candidate.fixed, &candidate.conditional)
                else {
                    continue;
                };
                for mut clause in match_chain(rest_chain, rest_ancestors) {
                    clause.extend(anc_conds.iter().cloned());
                    if clause.is_empty() {
                        return vec![Vec::new()];
                    }
                    clauses.push(clause);
                }
            }
            clauses
        }
        Combinator::AdjacentSibling | Combinator::SubsequentSibling => Vec::new(),
    }
}

fn wrap_condition_source(expr: &Expr, runtime: &TokenStream) -> TokenStream {
    match expr {
        Expr::Path(_) | Expr::Closure(_) | Expr::Lit(_) => {
            quote!(#runtime::view::IntoProp::<bool>::into_source(#expr))
        }
        other => quote!(#runtime::view::IntoProp::<bool>::into_source(move || (#other))),
    }
}

fn combine_dnf_condition(dnf: Vec<Vec<Expr>>, runtime: &TokenStream) -> TokenStream {
    let mut unique: Vec<(String, Expr)> = Vec::new();
    let mut index_of = |expr: &Expr| -> usize {
        let key = expr.to_token_stream().to_string();
        if let Some(pos) = unique.iter().position(|(k, _)| *k == key) {
            return pos;
        }
        unique.push((key, expr.clone()));
        unique.len() - 1
    };
    let clauses: Vec<TokenStream> = dnf
        .iter()
        .map(|clause| {
            let mut terms: Vec<TokenStream> = Vec::new();
            for expr in clause {
                let idx = index_of(expr);
                let ident = format_ident!("__nana_c{idx}");
                terms.push(quote!(#ident.get()));
            }
            match terms.as_slice() {
                [] => quote!(true),
                [one] => one.clone(),
                many => quote!((#(#many)&&*)),
            }
        })
        .collect();
    let bindings = unique.iter().enumerate().map(|(idx, (_, expr))| {
        let ident = format_ident!("__nana_c{idx}");
        let src = wrap_condition_source(expr, runtime);
        quote!(let #ident = #src;)
    });
    let body = match clauses.as_slice() {
        [] => quote!(false),
        [one] => one.clone(),
        many => quote!(#(#many)||*),
    };
    quote!({
        #(#bindings)*
        move || (#body)
    })
}

/// Turns the classes of a view's elements into `.class(…)` and
/// `.class_when(…)` on its sheet, the calls the Rust spelling writes.
pub(crate) struct Styler<'a> {
    sheet: &'a Sheet,
    name: syn::Ident,
    runtime: &'a TokenStream,
    classes: Vec<String>,
    ancestors: Vec<AncestorFrame>,
    /// Whether an element uses the sheet, so its statics are needed.
    used: bool,
    pub(crate) warnings: Vec<StyleWarning>,
}

impl<'a> Styler<'a> {
    fn new(
        sheet: &'a Sheet,
        classes: Vec<String>,
        name: syn::Ident,
        runtime: &'a TokenStream,
    ) -> Self {
        Self {
            sheet,
            name,
            runtime,
            classes,
            ancestors: Vec::new(),
            used: false,
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, span: Span, message: String) {
        self.warnings.push(StyleWarning {
            at: StyleAt::Template(span),
            message,
        });
    }

    pub(crate) fn nodes(&mut self, nodes: &mut [Node]) {
        for node in nodes {
            if let Node::Element(element) = node {
                self.element(element);
            }
        }
    }

    fn element(&mut self, element: &mut Element) {
        let tag = element.tag();
        let at_element = element.name.span();
        let mut fixed: Vec<(String, Span)> = Vec::new();
        let mut conditional: Vec<(String, Expr, Span)> = Vec::new();
        let mut kept = Vec::new();
        for attr in std::mem::take(&mut element.attrs) {
            match (&attr.name, &attr.value) {
                (AttrName::Plain(name), AttrValue::Lit(Expr::Lit(literal))) if name == "class" => {
                    if let Lit::Str(text) = &literal.lit {
                        for class in text.value().split_whitespace() {
                            fixed.push((class.to_owned(), name.span()));
                        }
                    }
                }
                (AttrName::Plain(name), _) if name == "class" => {
                    self.warn(
                        name.span(),
                        format!(
                            "`<{tag}>`: a bound `:class` is not compiled; write \
                             `class:name=\"condition\"` for each class"
                        ),
                    );
                }
                (AttrName::Directive(directive, span), AttrValue::Expr(condition))
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    conditional.push((class, condition.clone(), *span));
                }
                _ => kept.push(attr),
            }
        }

        let fixed_names: Vec<String> = fixed.iter().map(|(c, _)| c.clone()).collect();
        let cond_pairs: Vec<(String, Expr)> = conditional
            .iter()
            .map(|(c, e, _)| (c.clone(), e.clone()))
            .collect();

        let mut synth_fixed: Vec<(String, Span)> = Vec::new();
        let mut synth_conditional: Vec<(String, TokenStream, Span)> = Vec::new();
        if (!fixed_names.is_empty() || !cond_pairs.is_empty())
            && crate::is_builtin(&tag)
            && !BLOCKS.contains(&tag.as_str())
        {
            for target in &self.sheet.combinators {
                let Some(subj_conds) = match_compound(&target.subject, &fixed_names, &cond_pairs)
                else {
                    continue;
                };
                let anc_dnf = match_chain(&target.ancestors, &self.ancestors);
                if anc_dnf.is_empty() {
                    continue;
                }
                let combined: Vec<Vec<Expr>> = anc_dnf
                    .into_iter()
                    .map(|mut clause| {
                        clause.extend(subj_conds.iter().cloned());
                        clause
                    })
                    .collect();
                if combined.iter().any(|clause| clause.is_empty()) {
                    synth_fixed.push((target.synth_class.clone(), at_element));
                } else {
                    let expr_tokens = combine_dnf_condition(combined, self.runtime);
                    synth_conditional.push((target.synth_class.clone(), expr_tokens, at_element));
                }
            }
        }

        self.ancestors.push(AncestorFrame {
            fixed: fixed_names,
            conditional: cond_pairs,
        });
        self.nodes(&mut element.children);
        for attr in &mut kept {
            if let AttrValue::View(slot) = &mut attr.value {
                self.ancestors.push(AncestorFrame::default());
                self.nodes(slot);
                self.ancestors.pop();
            }
        }
        self.ancestors.pop();

        element.attrs = kept;
        if fixed.is_empty()
            && conditional.is_empty()
            && synth_fixed.is_empty()
            && synth_conditional.is_empty()
        {
            return;
        }
        if !crate::is_builtin(&tag) || BLOCKS.contains(&tag.as_str()) {
            self.warn(
                at_element,
                format!("`<{tag}>` takes no class: style the elements inside it"),
            );
            return;
        }
        if conditional.len() + synth_conditional.len() > MAX_CONDITIONAL_CLASSES {
            self.warn(
                at_element,
                format!("`<{tag}>` has more than {MAX_CONDITIONAL_CLASSES} conditional classes"),
            );
            return;
        }
        let all_fixed: Vec<String> = fixed
            .iter()
            .chain(synth_fixed.iter())
            .map(|(name, _)| name.clone())
            .collect();
        let all_conditional: Vec<String> = conditional
            .iter()
            .map(|(name, _, _)| name.clone())
            .chain(synth_conditional.iter().map(|(name, _, _)| name.clone()))
            .collect();
        let holds = |classes: &[String], conditional_too: bool| {
            classes.iter().all(|class| {
                all_fixed.iter().any(|name| name == class)
                    || (conditional_too && all_conditional.iter().any(|name| name == class))
            })
        };
        // The element follows one container on one axis: the `@container`
        // rules its classes can bring in must agree on it, with every
        // conditional class on.
        let queried: Vec<Result<ContainerQuery, ContainerQueryUnsupported>> = self
            .sheet
            .rules
            .iter()
            .filter(|rule| holds(&rule.classes, true))
            .filter_map(|rule| rule.query)
            .map(|index| Ok(self.sheet.queries[index].clone()))
            .collect();
        match plan_container_queries(&queried) {
            Err(ContainerPlanUnsupported::MixedContainers) => self.warn(
                at_element,
                format!(
                    "`<{tag}>`: its classes take `@container` rules on more than one container \
                     or axis; while such rules apply together, none of them does"
                ),
            ),
            Err(ContainerPlanUnsupported::TooManyBreakpoints) => self.warn(
                at_element,
                format!(
                    "`<{tag}>`: its classes take `@container` rules with more than \
                     {MAX_CONTAINER_BREAKPOINTS} breakpoints; while they all apply, none of \
                     them does"
                ),
            ),
            Ok(_) => {}
        }
        let runtime = self.runtime;
        let name = self.name.clone();
        let mut injected = Vec::new();
        let index = |classes: &[String], class: &str| {
            classes
                .iter()
                .position(|known| known == class)
                .map(|at| at as u16)
        };
        for (class, span) in fixed {
            match index(&self.classes, &class) {
                Some(at) => injected.push(Attr {
                    name: AttrName::Directive("class".into(), span),
                    value: AttrValue::Verbatim(quote!(#runtime::view::Class::new(&#name, #at))),
                }),
                None if self.sheet.referenced_classes.contains(&class) => {}
                None => self.warn(
                    span,
                    format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                ),
            }
        }
        for (class, span) in synth_fixed {
            if let Some(at) = index(&self.classes, &class) {
                injected.push(Attr {
                    name: AttrName::Directive("class".into(), span),
                    value: AttrValue::Verbatim(quote!(#runtime::view::Class::new(&#name, #at))),
                });
            }
        }
        for (class, condition, span) in conditional {
            let Some(at) = index(&self.classes, &class) else {
                if !self.sheet.referenced_classes.contains(&class) {
                    self.warn(
                        span,
                        format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                    );
                }
                continue;
            };
            let condition = match condition {
                Expr::Path(_) => quote!(#condition),
                condition => quote!(move || #condition),
            };
            injected.push(Attr {
                name: AttrName::Directive("class_when".into(), span),
                value: AttrValue::Verbatim(
                    quote!(#runtime::view::Class::new(&#name, #at), #condition),
                ),
            });
        }
        for (class, condition, span) in synth_conditional {
            if let Some(at) = index(&self.classes, &class) {
                injected.push(Attr {
                    name: AttrName::Directive("class_when".into(), span),
                    value: AttrValue::Verbatim(
                        quote!(#runtime::view::Class::new(&#name, #at), #condition),
                    ),
                });
            }
        }
        self.used |= !injected.is_empty();
        injected.append(&mut element.attrs);
        element.attrs = injected;
    }
}

/// What compiling a view's styles adds: statics to put before the view's
/// code, and warnings about what was not compiled.
pub struct CompiledStyles {
    pub items: Vec<TokenStream>,
    pub warnings: Vec<StyleWarning>,
}

/// Compile `css` against the elements of `nodes`: each element's
/// `class="…"` and `class:name="…"` become `.class(…)` and
/// `.class_when(…)` on the view's sheet, and are removed from the element.
pub fn compile_styles(css: &str, nodes: &mut [Node], runtime: &TokenStream) -> CompiledStyles {
    let sheet = parse_sheet(css, true);
    let name = format_ident!("__NANA_SHEET");
    let SheetItems { classes, items } = sheet.items(&name, runtime);
    let mut styler = Styler::new(&sheet, classes, name, runtime);
    styler.nodes(nodes);
    let mut warnings = sheet.warnings.clone();
    warnings.extend(styler.warnings);
    CompiledStyles {
        items: if styler.used { items } else { Vec::new() },
        warnings,
    }
}

/// `stylesheet! { mod name; … }`: a module holding the sheet and one
/// `view::Class` constant per class, named as the class with `-` as `_`.
/// `class_at` gives the span a class is written at: an unused class then
/// warns there.
pub fn compile_stylesheet(
    css: &str,
    module: &syn::Ident,
    public: bool,
    runtime: &TokenStream,
    class_at: &dyn Fn(&str) -> Span,
) -> (TokenStream, Vec<StyleWarning>) {
    let sheet = parse_sheet(css, false);
    let name = format_ident!("SHEET");
    let SheetItems { classes, items } = sheet.items(&name, runtime);
    let visibility = if public {
        quote!(pub)
    } else {
        quote!(pub(super))
    };
    let constants = classes.iter().enumerate().map(|(index, class)| {
        let ident = class.replace('-', "_");
        let span = class_at(class);
        let ident = match syn::parse_str::<syn::Ident>(&ident) {
            Ok(_) => syn::Ident::new(&ident, span),
            Err(_) => syn::Ident::new_raw(&ident, span),
        };
        let index = index as u16;
        quote! {
            #visibility const #ident: #runtime::view::Class = #runtime::view::Class::new(&#name, #index);
        }
    });
    let module_visibility = if public { quote!(pub) } else { quote!() };
    (
        quote! {
            #[allow(non_upper_case_globals)]
            #module_visibility mod #module {
                #(#items)*
                #(#constants)*
            }
        },
        sheet.warnings.clone(),
    )
}

/// `declarations` with each top-level at-rule (`@container … { … }`,
/// `@x …;`) blanked to spaces, the same length so other ranges still point
/// into the text, and the ranges blanked.
fn without_at_rules(declarations: &str) -> (String, Vec<Range<usize>>) {
    let bytes = declarations.as_bytes();
    let mut kept = bytes.to_vec();
    let mut ranges = Vec::new();
    let mut depth = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        if let Some(next) = skip_opaque(bytes, at) {
            at = next;
            continue;
        }
        match bytes[at] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'@' if depth == 0 => {
                let end = at_rule_end(bytes, at);
                kept[at..end].fill(b' ');
                ranges.push(at..end);
                at = end;
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    // Whole characters became spaces: the text is still UTF-8.
    let kept = String::from_utf8(kept).unwrap_or_default();
    (kept, ranges)
}

/// Where the at-rule starting at `start` ends: after its `;`, or after the
/// `}` closing its block, or at the end of the text.
fn at_rule_end(bytes: &[u8], start: usize) -> usize {
    let mut parens = 0usize;
    let mut braces = 0usize;
    let mut at = start;
    while at < bytes.len() {
        if let Some(next) = skip_opaque(bytes, at) {
            at = next;
            continue;
        }
        match bytes[at] {
            b'(' | b'[' => parens += 1,
            b')' | b']' => parens = parens.saturating_sub(1),
            b';' if parens == 0 && braces == 0 => return at + 1,
            b'{' if parens == 0 => braces += 1,
            b'}' if parens == 0 && braces > 0 => {
                braces -= 1;
                if braces == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

/// `css! { padding: 8px; transition: opacity 120ms }`: one declaration
/// block as a compiled style, an expression of type
/// `view::InlineStyle` for `El::css`. Warnings point into `declarations`.
/// An `@container` block in it is not compiled and is a warning: container
/// rules belong in a `<style>` or a `stylesheet!`.
pub fn compile_inline(
    declarations: &str,
    runtime: &TokenStream,
) -> (TokenStream, Vec<StyleWarning>) {
    const OPEN: &str = ".__nana_inline { ";
    // One declaration block holds no `@container` (or other at-rule): each
    // is blanked out, its offsets kept, and said so.
    let (kept, at_rules) = without_at_rules(declarations);
    let css = format!("{OPEN}{kept} }}");
    let sheet = parse_sheet(&css, false);
    // The wrapper is not the author's: what points at it points at the
    // whole block.
    let inner = |range: Range<usize>| {
        let clamp = |at: usize| at.saturating_sub(OPEN.len()).min(declarations.len());
        let range = clamp(range.start)..clamp(range.end);
        if range.is_empty() {
            0..declarations.len()
        } else {
            range
        }
    };
    let mut warnings: Vec<StyleWarning> = at_rules
        .into_iter()
        .map(|range| {
            let text = declarations[range.clone()].trim();
            let keyword = text
                .split(|c: char| c.is_whitespace() || c == '(' || c == '{' || c == ';')
                .next()
                .unwrap_or(text);
            StyleWarning {
                at: StyleAt::Sheet(range),
                message: format!(
                    "`{keyword}` is not compiled in `css!`, which holds one declaration block; \
                     write it in a `<style>` or a `stylesheet!`. It is ignored"
                ),
            }
        })
        .collect();
    warnings.extend(sheet.warnings.iter().cloned().map(|warning| StyleWarning {
        at: match warning.at {
            StyleAt::Sheet(range) => StyleAt::Sheet(inner(range)),
            StyleAt::Template(_) => StyleAt::Sheet(0..declarations.len()),
        },
        message: warning.message,
    }));
    let mut rules: Vec<&Rule> = sheet.rules.iter().collect();
    rules.sort_by_key(|rule| rule.rank);
    let patches = rules.iter().enumerate().map(|(index, rule)| {
        let name = format_ident!("__NANA_PATCH_{index}");
        let apply = patch_tokens(&rule.patch, runtime);
        quote! {
            static #name: #runtime::view::StylePatch = #runtime::view::StylePatch::new(#apply);
        }
    });
    let entries = (0..rules.len()).map(|index| {
        let name = format_ident!("__NANA_PATCH_{index}");
        quote!((0u64, &#name, ::core::option::Option::None))
    });
    let site = if rules.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote! {{
            static __NANA_SITE: #runtime::view::StyleSite =
                #runtime::view::StyleSite::new(&[#(#entries),*]);
            ::core::option::Option::Some(&__NANA_SITE)
        }}
    };
    let animate = sheet
        .transitions
        .iter()
        .max_by_key(|rule| rule.rank)
        .map_or_else(|| quote!([]), |rule| animate_tokens(&rule.items, runtime));
    (
        quote! {{
            #(#patches)*
            static __NANA_ANIMATE: &[#runtime::view::Implicit] = &#animate;
            #runtime::view::InlineStyle::new(#site, __NANA_ANIMATE)
        }},
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sheet warning as the text it points at and its message.
    fn located(css: &str) -> Vec<(&str, String)> {
        parse(css)
            .warnings
            .into_iter()
            .map(|warning| match warning.at {
                StyleAt::Sheet(range) => (&css[range], warning.message),
                StyleAt::Template(_) => unreachable!("a sheet alone has no template"),
            })
            .collect()
    }

    fn at<'a>(warnings: &[(&'a str, String)], needle: &str) -> &'a str {
        warnings
            .iter()
            .find(|(_, message)| message.contains(needle))
            .unwrap_or_else(|| panic!("no warning about {needle}: {warnings:?}"))
            .0
    }

    #[test]
    fn each_warning_points_at_what_it_is_about() {
        let warnings = located(
            "/* lead */ .a { padding: 4px; frobnicate: 3; transition: display 1s; animation: spin 1s }\n\
             @media (min-width: 1px) { .b { opacity: 1 } }\n\
             .c::before { opacity: 1 }\n\
             .d:hover .child { opacity: 0.5; }\n\
             div > .f { opacity: 1; }",
        );
        assert_eq!(at(&warnings, "frobnicate"), "frobnicate: 3");
        assert_eq!(at(&warnings, "not animated"), "transition: display 1s");
        assert_eq!(at(&warnings, "`@keyframes`"), "animation: spin 1s");
        assert_eq!(at(&warnings, "@media"), "@media (min-width: 1px)");
        assert_eq!(at(&warnings, "pseudo-element"), ".c::before");
        assert_eq!(at(&warnings, ":hover"), ".d:hover .child");
        assert_eq!(at(&warnings, "class selectors"), "div > .f");
        assert_eq!(warnings.len(), 7, "{warnings:?}");
    }

    /// What the engine skips while parsing is a warning at its text, not
    /// silently dropped.
    #[test]
    fn what_the_engine_skips_is_a_warning_at_its_text() {
        for (css, text, needle) in [
            (".a:has(> .b) { opacity: 0.5 }", ".a:has(> .b)", "selector"),
            (
                ".a::file-selector-button { opacity: 0.5 }",
                ".a::file-selector-button",
                "selector",
            ),
            (
                "@supports (frob: 1) { .a { opacity: 0.5 } }",
                "@supports (frob: 1)",
                "condition",
            ),
            (
                "@frobnicate x { .a { opacity: 0.5 } }",
                "@frobnicate x",
                "`@frobnicate`",
            ),
            ("@import \"x.css\";", "@import \"x.css\"", "`@import`"),
        ] {
            let warnings = located(css);
            assert_eq!(warnings.len(), 1, "{css}: {warnings:?}");
            assert_eq!(at(&warnings, needle), text, "{css}");
        }
        // One selector of a list, and what an `@supports` or `@layer` that
        // applies holds, are pointed at themselves.
        let warnings = located(
            ".a, .b:has(> .c) { opacity: 0.5 }\n\
             @supports (display: flex) { .d { opacity: 0.5 } @frob y { .e { opacity: 1 } } }\n\
             @layer base { .f::file-selector-button, .g { opacity: 0.5 } }",
        );
        let texts: Vec<&str> = warnings.iter().map(|(text, _)| *text).collect();
        assert_eq!(
            texts,
            [".b:has(> .c)", "@frob y", ".f::file-selector-button"],
            "{warnings:?}"
        );
        // A selector list warned about as a whole still points at the one
        // the engine skips.
        let warnings = located("div.a, .b:has(> .c) { opacity: 0.5 }");
        assert_eq!(at(&warnings, "class selectors"), "div.a, .b:has(> .c)");
        assert_eq!(at(&warnings, "CSS engine parses"), ".b:has(> .c)");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // An at-rule already warned about as a whole is not warned again.
        let warnings = located("@media (min-width: 1px) { @frob z { .a { opacity: 1 } } }");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(at(&warnings, "@media"), "@media (min-width: 1px)");
    }

    #[test]
    fn cascade_order_drops_unknown_properties_and_keeps_opacity_motion() {
        let sheet = parse(
            ".card { padding: 12px; opacity: 1; transition: opacity 150ms linear; }\n\
             .card.done { opacity: 0.5; }\n\
             .title { opacity: 0.9 !important; }\n\
             .title { opacity: 0.2; }\n\
             .card:hover { opacity: 0.7; }\n\
             .card > .title { padding: 2px; }\n\
             .card { frobnicate: 3; }",
        );
        let mut rules: Vec<&Rule> = sheet.rules.iter().collect();
        rules.sort_by_key(|rule| rule.rank);
        let classes: Vec<(Vec<&str>, u8)> = rules
            .iter()
            .map(|rule| {
                (
                    rule.classes.iter().map(String::as_str).collect(),
                    rule.pseudo,
                )
            })
            .collect();
        assert_eq!(
            classes,
            vec![
                (vec!["card"], 0),
                (vec!["title"], 0),
                (vec!["card", "done"], 0),
                (vec!["card"], PSEUDO_HOVER),
                (vec!["__nana_comb_0"], 0),
                (vec!["title"], 0),
            ]
        );
        assert!(
            !rules[1].rank.0,
            "the later normal title stays below important"
        );
        assert!(rules[5].rank.0, "the important title is last");
        assert!(
            rules.iter().all(|rule| !rule.patch.contains("frobnicate")),
            "an unknown property is not a patch"
        );
        let mut transitions: Vec<&TransitionRule> = sheet.transitions.iter().collect();
        transitions.sort_by_key(|rule| rule.rank);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].classes, ["card"]);
        assert!(
            transitions[0].items.iter().any(|item| {
                item.property == "Opacity"
                    && item.duration_ms == 150.0
                    && matches!(item.easing, Easing::Linear)
            }),
            "{:?}",
            transitions[0].items
        );
    }

    #[test]
    fn a_clean_sheet_warns_of_nothing() {
        let css =
            ".a { padding: 4px 8px; background: url(\"x;{y}.png\"); }\n.a.b { opacity: 0.5; }";
        assert!(located(css).is_empty(), "{:?}", located(css));
    }

    #[test]
    fn the_outline_skips_strings_comments_and_brackets() {
        let css =
            ".a[title=\"}{;\"] { content: \"a;b\"; /* ; } */ padding: 1px }\n@import \"x.css\";";
        let blocks = outline(css);
        assert_eq!(blocks.len(), 2);
        assert_eq!(&css[blocks[0].prelude.clone()], ".a[title=\"}{;\"]");
        let segments: Vec<_> = blocks[0]
            .segments
            .iter()
            .map(|segment| &css[segment.clone()])
            .collect();
        assert_eq!(segments, ["content: \"a;b\"", "padding: 1px"]);
        assert_eq!(&css[blocks[1].prelude.clone()], "@import \"x.css\"");
    }

    /// The rules of an `@container` block compile like the sheet's, with
    /// the block's query, and rank among the others by where they are
    /// written: a later plain rule still comes after them.
    #[test]
    fn container_rules_compile_with_their_query_in_cascade_order() {
        let sheet = parse(
            ".row { height: 20px; background: red; }\n\
             @container card (max-width: 300px) { .row { height: 40px; } .row.on { opacity: 0.5; } }\n\
             .row.on { height: 30px; }\n\
             @container card (max-width: 300px) { .row { padding: 2px; } }\n\
             @container style(--x: 1) { .row { width: 1px; } }",
        );
        assert_eq!(sheet.queries.len(), 1, "an equal query is one");
        let query = &sheet.queries[0];
        assert_eq!(query.name.as_deref(), Some("card"));
        assert_eq!(query.axis, ContainerAxis::Width);
        let mut rules: Vec<&Rule> = sheet.rules.iter().collect();
        rules.sort_by_key(|rule| rule.rank);
        let order: Vec<(Vec<&str>, Option<usize>, u32)> = rules
            .iter()
            .map(|rule| {
                (
                    rule.classes.iter().map(String::as_str).collect(),
                    rule.query,
                    rule.rank.2,
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (vec!["row"], None, 0),
                (vec!["row"], Some(0), 1),
                (vec!["row"], Some(0), 4),
                (vec!["row", "on"], Some(0), 2),
                (vec!["row", "on"], None, 3),
            ],
            "the unsupported block compiles nothing"
        );

        let items = sheet.items(&format_ident!("S"), &quote!(x));
        let tokens = items
            .items
            .iter()
            .map(ToString::to_string)
            .collect::<String>();
        assert_eq!(tokens.matches("SheetQuery =").count(), 1, "{tokens}");
        let bits = |value: f32| format!("from_bits ({}u32)", value.to_bits());
        assert!(tokens.contains(&bits(f32::NEG_INFINITY)), "{tokens}");
        assert!(
            tokens.contains(&bits(300f32.next_up())),
            "max-width includes 300: {tokens}"
        );
        assert!(tokens.contains("x :: ResponsiveAxis :: Width"), "{tokens}");
        assert_eq!(
            tokens
                .matches("query : :: core :: option :: Option :: Some")
                .count(),
            3
        );
    }

    /// Each `@container` problem is a warning at the CSS it is about; the
    /// container properties write fields and warn of nothing.
    #[test]
    fn container_warnings_point_at_what_they_are_about() {
        let warnings = located(
            ".a { container-type: inline-size; container-name: card; container: card / size; }\n\
             @container style(--wide: 1) { .b { opacity: 1 } }\n\
             @container (width > 10em) { .b { opacity: 1 } }\n\
             @container (width > 1px) {\n\
               div.c { opacity: 1 }\n\
               .e:hover .child { opacity: 1 }\n\
               .f { transition: opacity 1s; frobnicate: 1 }\n\
               @media (min-width: 1px) { .g { opacity: 1 } }\n\
               @container (height > 1px) { .h { opacity: 1 } }\n\
               .ok { opacity: 0.5 }\n\
             }\n\
             @supports (display: flex) { @container (inline-size: 1em) { .i { opacity: 1 } } }",
        );
        assert_eq!(at(&warnings, "`style()`"), "@container style(--wide: 1)");
        assert_eq!(at(&warnings, "`em`"), "@container (width > 10em)");
        assert_eq!(at(&warnings, "class selectors"), "div.c");
        assert_eq!(at(&warnings, ":hover"), ".e:hover .child");
        assert_eq!(
            at(&warnings, "`transition` inside `@container`"),
            "transition: opacity 1s"
        );
        assert_eq!(at(&warnings, "frobnicate"), "frobnicate: 1");
        assert_eq!(
            at(&warnings, "`@media` inside `@container`"),
            "@media (min-width: 1px)"
        );
        assert_eq!(
            at(&warnings, "inside another `@container`"),
            "@container (height > 1px)"
        );
        assert_eq!(
            at(&warnings, "a length in"),
            "@container (width > 10em)",
            "the first unit warning"
        );
        assert!(
            warnings
                .iter()
                .any(|(text, message)| *text == "@container (inline-size: 1em)"
                    && message.contains("`em`")),
            "an `@container` inside `@supports`: {warnings:?}"
        );
        assert_eq!(warnings.len(), 9, "{warnings:?}");
    }

    fn element(class: &str, conditional: &[&str]) -> Node {
        let mut attrs = vec![Attr {
            name: AttrName::Plain(format_ident!("class")),
            value: AttrValue::Lit(syn::parse_quote!(#class)),
        }];
        for name in conditional {
            attrs.push(Attr {
                name: AttrName::Directive(format!("class:{name}"), Span::call_site()),
                value: AttrValue::Expr(syn::parse_quote!(flag)),
            });
        }
        Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs,
            children: Vec::new(),
        })
    }

    /// An element follows one container on one axis: classes that bring in
    /// rules on two, or more than 16 bounds, warn at the element.
    #[test]
    fn an_element_whose_container_rules_cannot_share_a_plan_warns() {
        let css = "@container a (width < 100px) { .x { opacity: 0.5 } }\n\
                   @container b (width < 100px) { .y { opacity: 0.5 } }\n\
                   @container a (width < 200px) { .z { opacity: 0.5 } }";
        let warn = |nodes: &mut [Node]| {
            compile_styles(css, nodes, &quote!(x))
                .warnings
                .into_iter()
                .filter(|warning| warning.message.contains("more than one container"))
                .count()
        };
        assert_eq!(warn(&mut [element("x z", &[])]), 0, "one container");
        assert_eq!(warn(&mut [element("x", &["y"])]), 1, "with the class on");
        assert_eq!(warn(&mut [element("x y", &[])]), 1);

        let many: String = (1..=17)
            .map(|px| format!("@container (min-width: {px}px) {{ .m {{ opacity: 0.5 }} }}\n"))
            .collect();
        let compiled = compile_styles(&many, &mut [element("m", &[])], &quote!(x));
        assert!(
            compiled
                .warnings
                .iter()
                .any(|warning| warning.message.contains("breakpoints")),
            "{:?}",
            compiled
                .warnings
                .iter()
                .map(|warning| &warning.message)
                .collect::<Vec<_>>()
        );
    }

    /// `css!` holds one declaration block: an `@container` in it is a
    /// warning at it, and the declarations around it still compile.
    #[test]
    fn css_warns_of_an_at_rule_and_compiles_the_rest() {
        let declarations =
            "padding: 4px; @container (width < 300px) { padding: 2px; } opacity: 0.5";
        let (tokens, warnings) = compile_inline(declarations, &quote!(x));
        let pointed: Vec<(&str, &str)> = warnings
            .iter()
            .map(|warning| match &warning.at {
                StyleAt::Sheet(range) => (&declarations[range.clone()], warning.message.as_str()),
                StyleAt::Template(_) => unreachable!("css! has no template"),
            })
            .collect();
        assert_eq!(pointed.len(), 1, "{pointed:?}");
        assert_eq!(pointed[0].0, "@container (width < 300px) { padding: 2px; }");
        assert!(
            pointed[0]
                .1
                .contains("`@container` is not compiled in `css!`")
        );
        // Each written field compiles directly into a typed setter call.
        let tokens: String = tokens
            .to_string()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert!(
            tokens.contains("usex::view::__patchas__nana_patch;"),
            "{tokens}"
        );
        assert!(
            tokens.contains("__nana_patch::set::opacity(__nana_layout,Some(0.5_f32));"),
            "{tokens}"
        );
        assert!(
            tokens.contains(
                "__nana_patch::set::padding(__nana_layout,Some(__nana_patch::LengthSpec::Px(4.0_f32)));"
            ),
            "{tokens}"
        );
        assert!(!tokens.contains("__style_value"), "{tokens}");
        assert!(!tokens.contains("2.0_f32"), "{tokens}");
        assert!(!tokens.contains("SheetQuery"), "{tokens}");
    }

    #[test]
    fn complex_css_declarations_emit_valid_rust_patch_tokens() {
        let declarations = "\
            width: calc(50% - 8px); \
            max-width: min(76%, 620px); \
            background-image: linear-gradient(90deg, red, blue); \
            box-shadow: 0 2px 4px oklch(0.7 0.1 120); \
            grid-template-columns: [a] 1fr minmax(10px, 2fr); \
            transform: translate(4px, 8px); \
            padding: theme(space-md); \
            color: oklch(0.6 0.15 240);";
        let (tokens, warnings) = compile_inline(declarations, &quote!(x));
        assert!(warnings.is_empty(), "{warnings:?}");
        syn::parse2::<syn::Expr>(tokens.clone()).expect("compiled tokens are valid Rust expr");
        let compact: String = tokens
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(!compact.contains("__style_value"), "{compact}");
        assert!(
            compact.contains("usex::view::__patchas__nana_patch;"),
            "{compact}"
        );
        assert!(compact.contains("__nana_patch::set::width("), "{compact}");
        assert!(
            compact.contains("__nana_patch::set::max_width("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::paint::background_layers("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::paint::box_shadows("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::grid::columns("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::transforms::transform("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::theme::padding("),
            "{compact}"
        );
        assert!(
            compact.contains("__nana_patch::set::paint_colors::color("),
            "{compact}"
        );
    }

    #[test]
    fn removed_css_and_subgrid_diagnostics_cover_migration_cases() {
        let warnings = located(
            ".legacy { display: block; display: inline-block; display: flow-root; \
             display: contents; float: left; clear: both; }\n\
             .grid-bad { grid-template-columns: subgrid 80px; grid-template-rows: masonry; }\n\
             .grid-ok { display: grid; grid-template-columns: subgrid [a] [b]; \
             grid-template-rows: subgrid; }",
        );
        assert!(at(&warnings, "legacy block display").contains("display: block"));
        assert!(at(&warnings, "inline-block").contains("display: inline-block"));
        assert!(at(&warnings, "display: flow-root").contains("flow-root"));
        assert!(at(&warnings, "display: contents").contains("display: contents"));
        assert!(at(&warnings, "CSS floats").contains("float: left"));
        assert!(
            warnings
                .iter()
                .any(|(text, msg)| *text == "clear: both" && msg.contains("CSS floats"))
        );
        assert!(at(&warnings, "mixing `subgrid`").contains("subgrid 80px"));
        assert!(at(&warnings, "could not parse").contains("masonry"));
        assert!(
            !warnings
                .iter()
                .any(|(text, _)| text.contains("subgrid [a] [b]")
                    || *text == "grid-template-rows: subgrid"),
            "valid standalone subgrid must not emit warnings: {warnings:?}"
        );

        let (inline_tokens, inline_warnings) = compile_inline(
            "display: grid; grid-template-columns: subgrid; float: right;",
            &quote!(x),
        );
        assert!(
            inline_warnings
                .iter()
                .any(|w| w.message.contains("CSS floats")),
            "{inline_warnings:?}"
        );
        assert!(
            !inline_warnings
                .iter()
                .any(|w| w.message.contains("subgrid")),
            "{inline_warnings:?}"
        );
        let compact: String = inline_tokens
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            compact.contains("__nana_patch::set::grid::columns_subgrid(__nana_layout,true);"),
            "{compact}"
        );
    }

    #[test]
    fn dynamic_theme_tokens_emit_theme_aware_paint_colors() {
        let css = "\
            :root { --tint: #112233; }\n\
            :root[data-theme=\"dark\"] { --tint: #ddeeff; }\n\
            .box {\n\
                background: var(--nana-surface);\n\
                color: light-dark(#101010, #f0f0f0);\n\
                border-color: color-mix(in srgb, var(--nana-accent) 50%, var(--nana-surface));\n\
                outline-color: var(--tint);\n\
            }";
        let sheet = parse(css);
        assert!(sheet.warnings.is_empty(), "{:?}", sheet.warnings);
        let items = sheet.items(&format_ident!("S"), &quote!(x));
        let tokens: String = items
            .items
            .iter()
            .map(ToString::to_string)
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(tokens.contains("ThemeRole"), "{tokens}");
        assert!(tokens.contains("ThemeMix"), "{tokens}");
        assert!(tokens.contains("PaintColor::light_dark"), "{tokens}");
    }

    #[test]
    fn interactive_pseudos_emit_subject_bits_and_warn_on_ancestor_pseudo() {
        let css = "\
            .btn:hover { opacity: 0.9; transition: opacity 120ms 30ms linear; }\n\
            .btn:focus { opacity: 0.8; }\n\
            .btn:focus-visible { opacity: 0.7; }\n\
            .btn:active { opacity: 0.6; }\n\
            .card:hover .child { opacity: 0.5; }";
        let sheet = parse(css);
        assert_eq!(sheet.warnings.len(), 1, "{:?}", sheet.warnings);
        assert!(
            sheet.warnings[0].message.contains("subject"),
            "{:?}",
            sheet.warnings
        );
        let pseudos: Vec<u8> = sheet.rules.iter().map(|r| r.pseudo).collect();
        assert_eq!(
            pseudos,
            vec![
                PSEUDO_HOVER,
                PSEUDO_FOCUS,
                PSEUDO_FOCUS_VISIBLE,
                PSEUDO_ACTIVE
            ]
        );
        assert_eq!(sheet.transitions.len(), 1);
        assert_eq!(sheet.transitions[0].pseudo, PSEUDO_HOVER);
        assert_eq!(sheet.transitions[0].items[0].delay_ms, 30.0);
    }

    #[test]
    fn keyframes_and_animations_emit_with_valid_syntax_and_warn_when_unknown() {
        let css = "\
            @keyframes fade-shift {\n\
                from { opacity: 0.2; transform: translateX(0px); }\n\
                to { opacity: 1.0; transform: translateX(12px); color: light-dark(#111, #eee); filter: blur(2px); }\n\
            }\n\
            .card { animation: fade-shift 300ms 50ms steps(4, start) infinite alternate both; transition: filter 120ms; }\n\
            .card:hover { animation-play-state: paused; }\n\
            .bad { animation: missing 200ms; }";
        let sheet = parse(css);
        assert_eq!(sheet.warnings.len(), 1, "{:?}", sheet.warnings);
        assert!(
            sheet.warnings[0].message.contains("missing"),
            "{:?}",
            sheet.warnings
        );
        let items = sheet.items(&format_ident!("S"), &quote!(x));
        for item in &items.items {
            syn::parse2::<syn::Item>(item.clone()).expect("emitted animation item is valid Rust");
        }
        let tokens: String = items
            .items
            .iter()
            .map(ToString::to_string)
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(tokens.contains("Sheet::with_animations"), "{tokens}");
        assert!(tokens.contains("MotionCurve::Steps"), "{tokens}");
        assert!(tokens.contains("AnimationIteration::Infinite"), "{tokens}");
        assert!(tokens.contains("AnimationDirection::Alternate"), "{tokens}");
        assert!(tokens.contains("AnimationFillMode::Both"), "{tokens}");
        assert!(tokens.contains("AnimationPlayState::Paused"), "{tokens}");
        assert!(tokens.contains("CompiledMotionValue::Filter"), "{tokens}");
    }

    #[test]
    fn combinators_compile_in_styles_and_warn_in_stylesheet() {
        let css = "\
            .card.open > .title { opacity: 0.5; }\n\
            .card .badge { padding: 4px; }";
        let mut nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![
                Attr {
                    name: AttrName::Plain(format_ident!("class")),
                    value: AttrValue::Lit(syn::parse_quote!("card")),
                },
                Attr {
                    name: AttrName::Directive("class:open".into(), Span::call_site()),
                    value: AttrValue::Expr(syn::parse_quote!(open_sig)),
                },
            ],
            children: vec![
                element("title", &[]),
                Node::Element(Element {
                    module: Vec::new(),
                    name: format_ident!("Column"),
                    attrs: Vec::new(),
                    children: vec![element("badge", &[])],
                }),
            ],
        })];
        let compiled = compile_styles(css, &mut nodes, &quote!(x));
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        assert!(!compiled.items.is_empty());

        let (_, sheet_warnings) =
            compile_stylesheet(css, &format_ident!("m"), false, &quote!(x), &|_| {
                Span::call_site()
            });
        assert_eq!(sheet_warnings.len(), 2, "{sheet_warnings:?}");
    }
}
