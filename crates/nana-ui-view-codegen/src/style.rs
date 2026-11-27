//! A view's styles, compiled with `nana-ui-css` at build time: the `.vue`
//! compiler's `<style>`, `view!`'s `<style>`, inline `style="..."`,
//! `stylesheet!`, and `css!`.
//!
//! Built directly on `nana_ui_css::{parse_stylesheet, compile_declarations,
//! compile_inline_css, emit_style_op_rust, emit_media_query_list_rust,
//! emit_font_face_rule_rust, VarScope, CascadeKey}` and `nana_ui_core::StyleOp`.
//!
//! Features:
//! - Full CSS Cascade 5 `@layer` ordering (`CascadeKey`: layer order +
//!   `!important` reversal + specificity + source order)
//! - Inline `style="..."` on `.vue` / `view!` elements -> `.inline_patch(&PATCH)`
//!   (Layer 2)
//! - Pseudo-elements (`::placeholder`, `::selection`, `::-webkit-scrollbar*`)
//! - `@font-face` end-to-end (`Sheet::with_font_faces` + root `.sheet(&SHEET)`)
//! - Element-scoped static `--*` custom properties via hierarchical `VarScope`
//! - `@media` end-to-end (`Sheet::with_media` + `SheetMediaQuery`)
//! - `@container` queries (`SheetQuery` + breakpoint planning)
//! - Ancestor interactive selectors (`.card:hover .icon`, `.row:active .label`)
//!   via `interactive_state()` + `.track_interactive_state` + `.ancestor_pseudo`
//! - Tiered diagnostics (`DiagSeverity::Error` -> compile error,
//!   `DiagSeverity::Warning` -> warning; zero silent drops)
//! - Dev-only hot reload via `HotStylePatch::from_ops(ops)`.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::{Attr, AttrName, AttrValue, Element, Node};
use nana_ui_core::{
    AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState, ColorFilter,
    Easing, FontFaceRule, HotSheetAxis, HotSheetQuery, HotStylePatch, HotStyleRule, HotStyleSheet,
    LengthSpec, MediaCondition, MediaQuery, MediaQueryList, MediaType, MotionCurve, PaintColor,
    PaintTransform, ScrollbarSkin, StepJump, StyleOp,
};
use nana_ui_css::{
    CascadeKey, Combinator, CompileContext, CompiledAnimation, CompiledTransition, ComplexSelector,
    ComponentValue, CompoundSelector, ContainerAxis, ContainerPlanUnsupported, ContainerQuery,
    ContainerQueryUnsupported, CssDiag, CustomPropDecl, Declaration, DiagCode, DiagSeverity,
    KeyframesRule, LayerOrder, MAX_CONTAINER_BREAKPOINTS, ParsedSheet, PseudoClass, PseudoElement,
    Specificity, TypeSelector, VarScope, compile_declarations, compile_inline_css,
    emit_color_filter_rust, emit_font_face_rule_rust, emit_media_query_list_rust,
    emit_paint_color_rust, emit_style_op_rust, merge_scrollbar_skin, parse_stylesheet,
    parse_timing_function, plan_container_queries,
};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Lit};

const PSEUDO_HOVER: u8 = 1 << 0;
const PSEUDO_FOCUS: u8 = 1 << 1;
const PSEUDO_FOCUS_VISIBLE: u8 = 1 << 2;
const PSEUDO_ACTIVE: u8 = 1 << 3;
const PSEUDO_SELECTED: u8 = 1 << 4;
const MAX_CONDITIONAL_CLASSES: usize = 59;

/// Diagnostic severity tier exposed to macro and SFC frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleSeverity {
    Error,
    Warning,
}

/// What a style diagnostic points at.
#[derive(Debug, Clone)]
pub enum StyleAt {
    /// A byte range of the sheet's text (or `css!` declarations).
    Sheet(Range<usize>),
    /// An element of the template, or one of its attributes.
    Template(Span),
}

/// One compile-time style diagnostic (error or warning).
#[derive(Debug, Clone)]
pub struct StyleWarning {
    pub at: StyleAt,
    pub severity: StyleSeverity,
    pub message: String,
}

impl StyleWarning {
    fn sheet_warning(range: Range<usize>, message: impl Into<String>) -> Self {
        Self {
            at: StyleAt::Sheet(range),
            severity: StyleSeverity::Warning,
            message: message.into(),
        }
    }

    fn from_css_diag(diag: CssDiag) -> Self {
        Self {
            at: StyleAt::Sheet(diag.span.start as usize..diag.span.end as usize),
            severity: match diag.severity {
                DiagSeverity::Error => StyleSeverity::Error,
                DiagSeverity::Warning => StyleSeverity::Warning,
            },
            message: diag.message,
        }
    }

    fn from_template_css_diag(span: Span, diag: CssDiag) -> Self {
        Self {
            at: StyleAt::Template(span),
            severity: match diag.severity {
                DiagSeverity::Error => StyleSeverity::Error,
                DiagSeverity::Warning => StyleSeverity::Warning,
            },
            message: diag.message,
        }
    }
}

/// What compiling a view's styles adds: statics to put before the view's
/// code, warnings, and fatal CSS errors.
pub struct CompiledStyles {
    pub items: Vec<TokenStream>,
    pub warnings: Vec<StyleWarning>,
    pub errors: Vec<StyleWarning>,
}

/// What compiling a view's styles in hot mode adds, plus the pure-data
/// [`HotStyleSheet`] and structural `style_shape` token stream.
pub struct CompiledHotStyles {
    pub items: Vec<TokenStream>,
    pub warnings: Vec<StyleWarning>,
    pub errors: Vec<StyleWarning>,
    pub hot_sheet: HotStyleSheet,
    pub style_shape: TokenStream,
}

/// Result of compiling `stylesheet! { mod name; ... }`.
pub struct CompiledStylesheet {
    pub tokens: TokenStream,
    pub warnings: Vec<StyleWarning>,
    pub errors: Vec<StyleWarning>,
}

/// Result of compiling `css! { ... }`.
pub struct CompiledInlineStyle {
    pub tokens: TokenStream,
    pub warnings: Vec<StyleWarning>,
    pub errors: Vec<StyleWarning>,
}

// ============================================================================
// Internal IR
// ============================================================================

#[derive(Clone)]
struct Rule {
    classes: Vec<String>,
    pseudo: u8,
    rank: CascadeKey,
    /// Compiled `StyleOp`s (for `HotStylePatch::from_ops` and codegen).
    ops: Vec<StyleOp>,
    /// Rust setter statements targeting `__nana_patch`.
    patch: String,
    /// Optional `@container` query index into `Sheet::queries`.
    query: Option<usize>,
    /// Optional `@media` query index into `Sheet::media_queries`.
    media: Option<usize>,
    /// True if emitted from a `::-webkit-scrollbar*` pseudo-element rule.
    is_scrollbar_pseudo: bool,
}

#[derive(Clone, Debug)]
struct TransitionItem {
    property: &'static str,
    duration_ms: f32,
    delay_ms: f32,
    easing: Easing,
}

#[derive(Clone)]
struct TransitionRule {
    classes: Vec<String>,
    pseudo: u8,
    rank: CascadeKey,
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

#[derive(Clone)]
struct AnimationRule {
    classes: Vec<String>,
    pseudo: u8,
    rank: CascadeKey,
    items: Vec<CompiledAnimItem>,
}

#[derive(Clone, PartialEq, Eq)]
struct AncestorStep {
    combinator: Combinator,
    classes: Vec<String>,
    pseudo: u8,
}

#[derive(Clone)]
struct CombinatorTarget {
    /// Outermost -> closest-to-subject.
    ancestors: Vec<AncestorStep>,
    subject: Vec<String>,
    synth_class: String,
}

/// Custom property rule attached to an element-level selector (non-`:root`).
#[derive(Clone)]
struct ScopedVarRule {
    ancestors: Vec<AncestorStep>,
    subject: Vec<String>,
    layer: LayerOrder,
    specificity: Specificity,
    source_order: u32,
    custom_props: Vec<CustomPropDecl>,
}

/// Rule whose declarations reference at least one element-scoped `--*` variable
/// and is therefore specialized per element `VarScope` during the template walk.
#[derive(Clone)]
struct DeferredVarRule {
    ancestors: Vec<AncestorStep>,
    subject: Vec<String>,
    subject_pseudo: u8,
    pseudo_element: Option<PseudoElement>,
    rank: CascadeKey,
    declarations: Vec<Declaration>,
    query: Option<usize>,
    media: Option<usize>,
    span: Range<usize>,
}

pub(crate) struct Sheet {
    rules: Vec<Rule>,
    transitions: Vec<TransitionRule>,
    animations: Vec<AnimationRule>,
    combinators: Vec<CombinatorTarget>,
    scoped_var_rules: Vec<ScopedVarRule>,
    deferred_var_rules: Vec<DeferredVarRule>,
    referenced_classes: BTreeSet<String>,
    queries: Vec<ContainerQuery>,
    media_queries: Vec<MediaQueryList>,
    font_faces: Vec<FontFaceRule>,
    root_light: Arc<VarScope>,
    root_dark: Arc<VarScope>,
    compiled_kf: CompiledKeyframesMap,
    pub(crate) warnings: Vec<StyleWarning>,
    pub(crate) errors: Vec<StyleWarning>,
}

// ============================================================================
// Selector Classification
// ============================================================================

fn supported_pseudo_element(pe: &PseudoElement) -> bool {
    matches!(
        pe,
        PseudoElement::Placeholder
            | PseudoElement::Selection
            | PseudoElement::WebkitScrollbar
            | PseudoElement::WebkitScrollbarThumb
            | PseudoElement::WebkitScrollbarTrack
    )
}

fn interactive_pseudo_bit(p: &PseudoClass) -> Option<u8> {
    match p {
        PseudoClass::Hover => Some(PSEUDO_HOVER),
        PseudoClass::Focus => Some(PSEUDO_FOCUS),
        PseudoClass::FocusVisible => Some(PSEUDO_FOCUS_VISIBLE),
        PseudoClass::Active => Some(PSEUDO_ACTIVE),
        PseudoClass::Selected => Some(PSEUDO_SELECTED),
        _ => None,
    }
}

/// True if `sel` targets document root (`:root`, `html`, `body`, `*`,
/// `:root[data-theme="..."]`) so custom-property-only rules don't warn.
fn is_document_root_selector(sel: &ComplexSelector) -> bool {
    if !sel.rest.is_empty() {
        return false;
    }
    let comp = &sel.head;
    if comp.pseudo_element.is_some()
        || comp.id.is_some()
        || !comp.extra_ids.is_empty()
        || !comp.classes.is_empty()
    {
        return false;
    }
    if let Some(ts) = &comp.type_sel {
        match ts {
            TypeSelector::Universal => {}
            TypeSelector::Tag(tag)
                if tag.eq_ignore_ascii_case("html") || tag.eq_ignore_ascii_case("body") => {}
            _ => return false,
        }
    }
    if !comp.pseudos.iter().all(|p| matches!(p, PseudoClass::Root)) {
        return false;
    }
    if !comp
        .attrs
        .iter()
        .all(|a| a.name.eq_ignore_ascii_case("data-theme"))
    {
        return false;
    }
    !comp.is_empty()
}

struct ClassifiedSelector {
    ancestors: Vec<AncestorStep>,
    subject: Vec<String>,
    subject_pseudo: u8,
    pseudo_element: Option<PseudoElement>,
}

enum SelectorCheck {
    Supported(ClassifiedSelector),
    /// Handled as document-root custom property rule; skip silently.
    DocumentRoot,
    /// Unsupported by build-time class matcher; report warning with message.
    Unsupported(String),
}

fn check_compound(
    comp: &CompoundSelector,
    is_subject: bool,
) -> Result<(Vec<String>, u8, Option<PseudoElement>), String> {
    if let Some(pe) = &comp.pseudo_element {
        if !is_subject {
            return Err("pseudo-elements are only supported on the subject selector".into());
        }
        if !supported_pseudo_element(pe) {
            return Err(format!(
                "pseudo-element `{pe:?}` is not supported in NanaUI's Style Model"
            ));
        }
    }
    if comp.type_sel.is_some()
        || comp.id.is_some()
        || !comp.extra_ids.is_empty()
        || !comp.attrs.is_empty()
        || comp.classes.is_empty()
    {
        return Err(
            "only class selectors (`.name`, `.a.b`, `.a > .b`, `.a .b`) are matched at build time"
                .into(),
        );
    }
    let mut pseudo_mask = 0u8;
    for p in &comp.pseudos {
        if let Some(bit) = interactive_pseudo_bit(p) {
            pseudo_mask |= bit;
        } else {
            return Err(
                "only `:hover`, `:focus`, `:focus-visible`, `:active`, and `:selected` \
                 pseudo-classes are supported on class selectors"
                    .into(),
            );
        }
    }
    Ok((
        comp.classes.clone(),
        pseudo_mask,
        comp.pseudo_element.clone(),
    ))
}

fn classify_selector(
    sel: &ComplexSelector,
    allow_combinators: bool,
    custom_props_only: bool,
) -> SelectorCheck {
    if custom_props_only && is_document_root_selector(sel) {
        return SelectorCheck::DocumentRoot;
    }
    if sel.rest.is_empty() {
        return match check_compound(&sel.head, true) {
            Ok((subject, subject_pseudo, pseudo_element)) => {
                SelectorCheck::Supported(ClassifiedSelector {
                    ancestors: Vec::new(),
                    subject,
                    subject_pseudo,
                    pseudo_element,
                })
            }
            Err(msg) => SelectorCheck::Unsupported(msg),
        };
    }

    if !allow_combinators {
        return SelectorCheck::Unsupported(
            "combinator selectors are only matched inside a template's `<style>`, not `stylesheet!`"
                .into(),
        );
    }

    if sel.has_sibling_combinator() {
        return SelectorCheck::Unsupported(
            "sibling combinators (`+`, `~`) are not supported; only child (`>`) and descendant \
             (` `) combinators are matched"
                .into(),
        );
    }

    // Build outermost -> closest-to-subject ancestor chain + subject.
    // `sel.head` is outermost; `sel.rest[i]` is `(combinator_between_i_and_i+1, compound_i+1)`.
    let mut all_compounds: Vec<&CompoundSelector> = Vec::with_capacity(sel.rest.len() + 1);
    all_compounds.push(&sel.head);
    let mut combinators: Vec<Combinator> = Vec::with_capacity(sel.rest.len());
    for (comb, comp) in &sel.rest {
        combinators.push(*comb);
        all_compounds.push(comp);
    }

    let mut ancestors = Vec::with_capacity(combinators.len());
    for (idx, comb) in combinators.into_iter().enumerate() {
        let (classes, pseudo, _) = match check_compound(all_compounds[idx], false) {
            Ok(v) => v,
            Err(msg) => return SelectorCheck::Unsupported(msg),
        };
        ancestors.push(AncestorStep {
            combinator: comb,
            classes,
            pseudo,
        });
    }

    let (subject, subject_pseudo, pseudo_element) =
        match check_compound(all_compounds.last().expect("non-empty"), true) {
            Ok(v) => v,
            Err(msg) => return SelectorCheck::Unsupported(msg),
        };

    SelectorCheck::Supported(ClassifiedSelector {
        ancestors,
        subject,
        subject_pseudo,
        pseudo_element,
    })
}

fn normalize_class_list(mut classes: Vec<String>) -> Vec<String> {
    classes.sort();
    classes.dedup();
    classes
}

fn intern_combinator(
    combinators: &mut Vec<CombinatorTarget>,
    referenced: &mut BTreeSet<String>,
    ancestors: Vec<AncestorStep>,
    subject: Vec<String>,
) -> String {
    let ancestors: Vec<AncestorStep> = ancestors
        .into_iter()
        .map(|step| {
            for c in &step.classes {
                referenced.insert(c.clone());
            }
            AncestorStep {
                combinator: step.combinator,
                classes: normalize_class_list(step.classes),
                pseudo: step.pseudo,
            }
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

// ============================================================================
// Dual Light/Dark Compilation & Variable Reference Tracking
// ============================================================================

fn collect_var_refs(values: &[ComponentValue], out: &mut BTreeSet<String>) {
    for cv in values {
        match cv {
            ComponentValue::Function { name, args, .. } => {
                if name.eq_ignore_ascii_case("var") {
                    for arg in args {
                        if let Some(id) = arg.as_ident()
                            && id.starts_with("--")
                        {
                            out.insert(id.to_owned());
                            break;
                        }
                    }
                }
                collect_var_refs(args, out);
            }
            ComponentValue::SimpleBlock { values, .. } => {
                collect_var_refs(values, out);
            }
            ComponentValue::Token(_) => {}
        }
    }
}

fn decls_reference_any_var(decls: &[Declaration], names: &BTreeSet<String>) -> bool {
    if names.is_empty() {
        return false;
    }
    let mut refs = BTreeSet::new();
    for d in decls {
        collect_var_refs(&d.value, &mut refs);
    }
    refs.iter().any(|r| names.contains(r))
}

fn merge_paint_color(light: PaintColor, dark: PaintColor) -> PaintColor {
    if light == dark {
        light
    } else {
        PaintColor::light_dark(light, dark)
    }
}

fn merge_dual_style_ops(light_ops: Vec<StyleOp>, dark_ops: Vec<StyleOp>) -> Vec<StyleOp> {
    if light_ops == dark_ops || light_ops.len() != dark_ops.len() {
        return light_ops;
    }
    light_ops
        .into_iter()
        .zip(dark_ops)
        .map(|(l, d)| match (l, d) {
            (StyleOp::Color(Some(lc)), StyleOp::Color(Some(dc))) => {
                StyleOp::Color(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::Background(Some(lc)), StyleOp::Background(Some(dc))) => {
                StyleOp::Background(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::BorderColor(Some(lc)), StyleOp::BorderColor(Some(dc))) => {
                StyleOp::BorderColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::BorderTopColor(Some(lc)), StyleOp::BorderTopColor(Some(dc))) => {
                StyleOp::BorderTopColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::BorderRightColor(Some(lc)), StyleOp::BorderRightColor(Some(dc))) => {
                StyleOp::BorderRightColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::BorderBottomColor(Some(lc)), StyleOp::BorderBottomColor(Some(dc))) => {
                StyleOp::BorderBottomColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::BorderLeftColor(Some(lc)), StyleOp::BorderLeftColor(Some(dc))) => {
                StyleOp::BorderLeftColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::OutlineColor(Some(lc)), StyleOp::OutlineColor(Some(dc))) => {
                StyleOp::OutlineColor(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::SelectionBackground(Some(lc)), StyleOp::SelectionBackground(Some(dc))) => {
                StyleOp::SelectionBackground(Some(merge_paint_color(lc, dc)))
            }
            (StyleOp::SelectionColor(Some(lc)), StyleOp::SelectionColor(Some(dc))) => {
                StyleOp::SelectionColor(Some(merge_paint_color(lc, dc)))
            }
            (other_l, _) => other_l,
        })
        .collect()
}

fn compile_decls_dual(
    decls: &[Declaration],
    vars_light: &VarScope,
    vars_dark: &VarScope,
    pseudo: Option<&PseudoElement>,
    diags: &mut Vec<CssDiag>,
) -> (
    Vec<StyleOp>,
    Vec<CompiledTransition>,
    Vec<CompiledAnimation>,
) {
    let ctx = CompileContext::default();
    let light = compile_declarations(decls, vars_light, pseudo, &ctx, diags);
    // Compile against dark scope as well to support `:root[data-theme="dark"]`
    // custom properties lowering into `PaintColor::light_dark(...)`.
    let mut dark_diags = Vec::new();
    let dark = compile_declarations(decls, vars_dark, pseudo, &ctx, &mut dark_diags);
    let ops = merge_dual_style_ops(light.ops, dark.ops);
    (ops, light.transitions, light.animations)
}

fn compile_inline_dual(
    css: &str,
    vars_light: &VarScope,
    vars_dark: &VarScope,
) -> (Vec<StyleOp>, Vec<CompiledTransition>, Vec<CssDiag>) {
    let ctx = CompileContext::default();
    let light = compile_inline_css(css, vars_light, &ctx);
    let dark = compile_inline_css(css, vars_dark, &ctx);
    let ops = merge_dual_style_ops(light.ops, dark.ops);
    (ops, light.transitions, light.diags)
}

// ============================================================================
// Motion & Keyframes
// ============================================================================

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

fn easing_from_curve(curve: MotionCurve) -> Easing {
    match curve {
        MotionCurve::Easing(e) => e,
        _ => Easing::Linear,
    }
}

fn transition_items_from_compiled(
    compiled: &[CompiledTransition],
    span: Range<usize>,
    warnings: &mut Vec<StyleWarning>,
) -> Vec<TransitionItem> {
    let mut items = Vec::new();
    for tr in compiled {
        let variants = animatable(&tr.property);
        if variants.is_empty() {
            warnings.push(StyleWarning::sheet_warning(
                span.clone(),
                format!(
                    "`transition: {}` is not animated: only opacity, transform, filter, \
                     width, height, background and color are",
                    tr.property
                ),
            ));
            continue;
        }
        let easing = easing_from_curve(tr.curve);
        for &variant in variants {
            items.push(TransitionItem {
                property: variant,
                duration_ms: tr.duration_ms,
                delay_ms: tr.delay_ms,
                easing,
            });
        }
    }
    items
}

type CompiledKeyframesMap = BTreeMap<String, BTreeMap<&'static str, Vec<CompiledKfStop>>>;

fn extract_keyframe_props(ops: &[StyleOp]) -> Vec<(&'static str, CompiledKfValue)> {
    let mut values: BTreeMap<&'static str, CompiledKfValue> = BTreeMap::new();
    let mut has_3d_or_unsupported = false;
    for op in ops {
        match op {
            StyleOp::Transform3d(Some(_)) | StyleOp::UnsupportedTransform(Some(_)) => {
                has_3d_or_unsupported = true;
                values.remove("Transform");
            }
            StyleOp::Opacity(Some(v)) => {
                values.insert("Opacity", CompiledKfValue::Scalar(*v));
            }
            StyleOp::Transform(t) if !has_3d_or_unsupported => {
                values.insert(
                    "Transform",
                    CompiledKfValue::Transform(t.unwrap_or_default()),
                );
            }
            StyleOp::Background(bg) => {
                values.insert(
                    "Background",
                    CompiledKfValue::Color(bg.unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 0.0]))),
                );
            }
            StyleOp::Color(Some(c)) => {
                values.insert("Color", CompiledKfValue::Color(*c));
            }
            StyleOp::Filter(f) => {
                values.insert(
                    "Filter",
                    CompiledKfValue::Filter(f.clone().unwrap_or_default()),
                );
            }
            StyleOp::Width(Some(LengthSpec::Px(px))) => {
                values.insert("Width", CompiledKfValue::Scalar(*px));
            }
            StyleOp::Height(Some(LengthSpec::Px(px))) => {
                values.insert("Height", CompiledKfValue::Scalar(*px));
            }
            StyleOp::Padding(Some(LengthSpec::Px(px))) => {
                values.insert("Padding", CompiledKfValue::Scalar(*px));
            }
            StyleOp::Margin(Some(LengthSpec::Px(px))) => {
                values.insert("Margin", CompiledKfValue::Scalar(*px));
            }
            _ => {}
        }
    }
    values.into_iter().collect()
}

fn compile_keyframes_map(
    keyframes: &BTreeMap<String, KeyframesRule>,
    vars_light: &VarScope,
    vars_dark: &VarScope,
    diags: &mut Vec<CssDiag>,
) -> CompiledKeyframesMap {
    let mut compiled = CompiledKeyframesMap::new();
    for (name, rule) in keyframes {
        let mut by_offset: BTreeMap<u32, (f32, Vec<&Declaration>)> = BTreeMap::new();
        for block in &rule.blocks {
            for stop in &block.stops {
                let offset = stop.as_progress();
                let key = (offset * 10_000.0).round() as u32;
                let entry = by_offset.entry(key).or_insert_with(|| (offset, Vec::new()));
                entry.1.extend(block.declarations.iter());
            }
        }
        let mut tracks: BTreeMap<&'static str, Vec<CompiledKfStop>> = BTreeMap::new();
        for (offset, entries) in by_offset.into_values() {
            let mut stop_easing = None;
            let mut decls = Vec::new();
            for decl in entries {
                if decl.name.starts_with("--") {
                    continue;
                }
                if decl.name.eq_ignore_ascii_case("animation-timing-function") {
                    if let Some(MotionCurve::Easing(e)) = parse_timing_function(&decl.value) {
                        stop_easing = Some(e);
                    }
                    continue;
                }
                decls.push(decl.clone());
            }
            if decls.is_empty() {
                continue;
            }
            let (ops, _, _) = compile_decls_dual(&decls, vars_light, vars_dark, None, diags);
            for (property, value) in extract_keyframe_props(&ops) {
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

fn animation_items_from_compiled(
    compiled: &[CompiledAnimation],
    compiled_kf: &CompiledKeyframesMap,
    span: Range<usize>,
    warnings: &mut Vec<StyleWarning>,
) -> Vec<CompiledAnimItem> {
    let mut out = Vec::new();
    for anim in compiled {
        if anim.name.is_empty() {
            // Standalone `animation-play-state: paused|running`.
            out.push(CompiledAnimItem {
                name: String::new(),
                property: "Opacity",
                stops: Vec::new(),
                duration_ms: 0.0,
                delay_ms: 0.0,
                curve: MotionCurve::Easing(Easing::Linear),
                iterations: AnimationIteration::ONCE,
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::None,
                play_state: anim.play_state,
            });
            continue;
        }
        if anim.name.eq_ignore_ascii_case("none") {
            out.push(CompiledAnimItem {
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
            });
            continue;
        }
        let Some(tracks) = compiled_kf.get(&anim.name) else {
            warnings.push(StyleWarning::sheet_warning(
                span.clone(),
                format!("`animation`: unknown `@keyframes` `{}`", anim.name),
            ));
            continue;
        };
        if tracks.is_empty() {
            warnings.push(StyleWarning::sheet_warning(
                span.clone(),
                format!(
                    "`animation: {}`: `@keyframes {}` sets no animatable property",
                    anim.name, anim.name
                ),
            ));
            continue;
        }
        for (&property, stops) in tracks {
            out.push(CompiledAnimItem {
                name: anim.name.clone(),
                property,
                stops: stops.clone(),
                duration_ms: anim.duration_ms,
                delay_ms: anim.delay_ms,
                curve: anim.curve,
                iterations: anim.iterations,
                direction: anim.direction,
                fill_mode: anim.fill_mode,
                play_state: anim.play_state,
            });
        }
    }
    out
}

// ============================================================================
// Media Query Intersection (for nested `@media`)
// ============================================================================

fn intersect_media_lists(lists: &[MediaQueryList]) -> Option<MediaQueryList> {
    if lists.is_empty() {
        return None;
    }
    let mut acc = lists[0].clone();
    for next in &lists[1..] {
        let mut combined = Vec::new();
        for a in &acc.queries {
            for b in &next.queries {
                if let Some(q) = intersect_media_queries(a, b) {
                    combined.push(q);
                }
            }
        }
        acc = MediaQueryList { queries: combined };
    }
    Some(acc)
}

fn query_as_condition(q: &MediaQuery) -> Option<(MediaType, Option<MediaCondition>)> {
    if !q.negated {
        return Some((q.media_type, q.condition.clone()));
    }
    // In NanaUI's screen environment, `not all and (C)` / `not screen and (C)`
    // negates the condition; `not print` matches screen unconditionally.
    match q.media_type {
        MediaType::All | MediaType::Screen => match &q.condition {
            Some(c) => Some((MediaType::Screen, Some(MediaCondition::negate(c.clone())))),
            None => None,
        },
        MediaType::Print | MediaType::Other => Some((MediaType::Screen, None)),
    }
}

fn intersect_media_queries(a: &MediaQuery, b: &MediaQuery) -> Option<MediaQuery> {
    let (ta, ca) = query_as_condition(a)?;
    let (tb, cb) = query_as_condition(b)?;
    let media_type = match (ta, tb) {
        (MediaType::All, other) | (other, MediaType::All) => other,
        (x, y) if x == y => x,
        _ => return None,
    };
    let condition = match (ca, cb) {
        (None, c) | (c, None) => c,
        (Some(c1), Some(c2)) => Some(MediaCondition::and(vec![c1, c2])),
    };
    Some(MediaQuery::with_type(false, media_type, condition))
}

// ============================================================================
// StyleOp -> Patch Rust Statements (with ScrollbarSkin merging)
// ============================================================================

fn emit_f32_lit(v: f32) -> String {
    if v.is_nan() {
        "f32::NAN".into()
    } else if v.is_infinite() {
        if v.is_sign_positive() {
            "f32::INFINITY".into()
        } else {
            "f32::NEG_INFINITY".into()
        }
    } else {
        format!("{v:?}_f32")
    }
}

fn emit_rgba_lit(c: [f32; 4]) -> String {
    format!(
        "[{}, {}, {}, {}]",
        emit_f32_lit(c[0]),
        emit_f32_lit(c[1]),
        emit_f32_lit(c[2]),
        emit_f32_lit(c[3]),
    )
}

fn emit_scrollbar_merge_rust(skin: &ScrollbarSkin, patch_root: &str, out: &mut String) {
    out.push_str("{ let mut __nana_sb = __nana_layout.paint.scrollbar.unwrap_or_default();");
    if let Some(v) = skin.thickness {
        out.push_str(&format!(
            " __nana_sb.thickness = ::core::option::Option::Some({});",
            emit_f32_lit(v)
        ));
    }
    if let Some(v) = skin.thumb_thickness {
        out.push_str(&format!(
            " __nana_sb.thumb_thickness = ::core::option::Option::Some({});",
            emit_f32_lit(v)
        ));
    }
    if let Some(v) = skin.thumb_active_thickness {
        out.push_str(&format!(
            " __nana_sb.thumb_active_thickness = ::core::option::Option::Some({});",
            emit_f32_lit(v)
        ));
    }
    if let Some(c) = skin.track_color {
        out.push_str(&format!(
            " __nana_sb.track_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if let Some(c) = skin.track_hot_color {
        out.push_str(&format!(
            " __nana_sb.track_hot_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if let Some(c) = skin.thumb_color {
        out.push_str(&format!(
            " __nana_sb.thumb_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if let Some(c) = skin.thumb_hot_color {
        out.push_str(&format!(
            " __nana_sb.thumb_hot_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if let Some(c) = skin.track_border_color {
        out.push_str(&format!(
            " __nana_sb.track_border_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if let Some(c) = skin.thumb_border_color {
        out.push_str(&format!(
            " __nana_sb.thumb_border_color = ::core::option::Option::Some({});",
            emit_rgba_lit(c)
        ));
    }
    if skin.over_media {
        out.push_str(" __nana_sb.over_media = true;");
    }
    if let Some(v) = skin.room {
        out.push_str(&format!(
            " __nana_sb.room = ::core::option::Option::Some({});",
            emit_f32_lit(v)
        ));
    }
    out.push_str(&format!(
        " {patch_root}::set::paint::scrollbar(__nana_layout, ::core::option::Option::Some(__nana_sb)); }}"
    ));
}

fn emit_patch_stmts(ops: &[StyleOp], patch_root: &str, merge_scrollbar: bool) -> String {
    let mut out = String::new();
    for op in ops {
        if merge_scrollbar && let StyleOp::Scrollbar(Some(skin)) = op {
            emit_scrollbar_merge_rust(skin, patch_root, &mut out);
        } else {
            emit_style_op_rust(op, patch_root, &mut out);
        }
    }
    out
}

// ============================================================================
// Stylesheet Parsing & Rule Lowering
// ============================================================================

fn collect_inline_scoped_var_names(nodes: &[Node], out: &mut BTreeSet<String>) {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        if element.module.is_empty() && element.name != "Span" && crate::is_builtin(&element.tag())
        {
            for attr in &element.attrs {
                if let (
                    AttrName::Plain(name),
                    AttrValue::Lit(Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(text),
                        ..
                    })),
                ) = (&attr.name, &attr.value)
                    && name == "style"
                {
                    let compiled = compile_inline_css(
                        &text.value(),
                        &VarScope::new(),
                        &CompileContext::default(),
                    );
                    for cp in compiled.custom_props {
                        out.insert(cp.name);
                    }
                }
            }
        }
        for attr in &element.attrs {
            if let AttrValue::View(slot) = &attr.value {
                collect_inline_scoped_var_names(slot, out);
            }
        }
        collect_inline_scoped_var_names(&element.children, out);
    }
}

fn expand_transitive_scoped_vars(
    root_light: &VarScope,
    root_dark: &VarScope,
    scoped: &mut BTreeSet<String>,
) {
    if scoped.is_empty() {
        return;
    }
    loop {
        let before = scoped.len();
        for scope in [root_light, root_dark] {
            for (name, val) in scope.local_entries() {
                if scoped.contains(name) {
                    continue;
                }
                let mut refs = BTreeSet::new();
                collect_var_refs(val, &mut refs);
                if refs.iter().any(|r| scoped.contains(r)) {
                    scoped.insert(name.to_owned());
                }
            }
        }
        if scoped.len() == before {
            break;
        }
    }
}

fn push_diags(raw: Vec<CssDiag>, warnings: &mut Vec<StyleWarning>, errors: &mut Vec<StyleWarning>) {
    for diag in raw {
        let item = StyleWarning::from_css_diag(diag);
        match item.severity {
            StyleSeverity::Error => errors.push(item),
            StyleSeverity::Warning => warnings.push(item),
        }
    }
}

fn push_template_diags(
    span: Span,
    raw: Vec<CssDiag>,
    warnings: &mut Vec<StyleWarning>,
    errors: &mut Vec<StyleWarning>,
) {
    for diag in raw {
        let item = StyleWarning::from_template_css_diag(span, diag);
        match item.severity {
            StyleSeverity::Error => errors.push(item),
            StyleSeverity::Warning => warnings.push(item),
        }
    }
}

#[cfg(test)]
pub(crate) fn parse(css: &str) -> Sheet {
    parse_sheet(css, true, BTreeSet::new())
}

pub(crate) fn parse_sheet(
    css: &str,
    allow_combinators: bool,
    extra_scoped_vars: BTreeSet<String>,
) -> Sheet {
    let parsed: ParsedSheet = parse_stylesheet(css);
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    push_diags(parsed.diags.clone(), &mut warnings, &mut errors);

    let root_light = parsed.root_var_scope("light");
    let root_dark = parsed.root_var_scope("dark");

    let mut kf_diags = Vec::new();
    let compiled_kf =
        compile_keyframes_map(&parsed.keyframes, &root_light, &root_dark, &mut kf_diags);
    push_diags(kf_diags, &mut warnings, &mut errors);

    // Collect element-scoped custom property names declared on non-root rules
    // (plus any defined in inline `style="..."` attributes).
    let mut scoped_var_names = extra_scoped_vars;
    let mut scoped_var_rules: Vec<ScopedVarRule> = Vec::new();
    if allow_combinators {
        for rule in &parsed.rules {
            if rule.custom_props.is_empty() || !rule.media.is_empty() || rule.container.is_some() {
                continue;
            }
            for sel in &rule.selectors.0 {
                if is_document_root_selector(sel) {
                    continue;
                }
                if let SelectorCheck::Supported(classified) =
                    classify_selector(sel, allow_combinators, true)
                    && classified.subject_pseudo == 0
                    && classified.pseudo_element.is_none()
                    && classified.ancestors.iter().all(|a| a.pseudo == 0)
                {
                    for cp in &rule.custom_props {
                        scoped_var_names.insert(cp.name.clone());
                    }
                    scoped_var_rules.push(ScopedVarRule {
                        ancestors: classified.ancestors,
                        subject: classified.subject,
                        layer: rule.layer,
                        specificity: sel.specificity(),
                        source_order: rule.source_order,
                        custom_props: rule.custom_props.clone(),
                    });
                }
            }
        }
        expand_transitive_scoped_vars(&root_light, &root_dark, &mut scoped_var_names);
    }

    let mut rules: Vec<Rule> = Vec::new();
    let mut transitions: Vec<TransitionRule> = Vec::new();
    let mut animations: Vec<AnimationRule> = Vec::new();
    let mut combinators: Vec<CombinatorTarget> = Vec::new();
    let mut deferred_var_rules: Vec<DeferredVarRule> = Vec::new();
    let mut referenced_classes: BTreeSet<String> = BTreeSet::new();
    let mut queries: Vec<ContainerQuery> = Vec::new();
    let mut media_queries: Vec<MediaQueryList> = Vec::new();
    let mut warned_container_spans: BTreeSet<(usize, usize)> = BTreeSet::new();

    for rule in &parsed.rules {
        let rule_range = rule.span.start as usize..rule.span.end as usize;

        // Handle `@container` guard.
        let query_idx = match &rule.container {
            None => None,
            Some(Ok(q)) => {
                let idx = match queries.iter().position(|known| known == q) {
                    Some(i) => i,
                    None => {
                        queries.push(q.clone());
                        queries.len() - 1
                    }
                };
                Some(idx)
            }
            Some(Err(unsupported)) => {
                if warned_container_spans.insert((rule.span.start, rule.span.end)) {
                    warnings.push(StyleWarning::sheet_warning(
                        rule_range.clone(),
                        format!(
                            "`@container` query uses {unsupported}, which is not evaluated; \
                             its rules are ignored"
                        ),
                    ));
                }
                continue;
            }
        };

        // Handle `@media` guard(s).
        let media_idx = match intersect_media_lists(&rule.media) {
            None => None,
            Some(list) => {
                let idx = match media_queries.iter().position(|known| known == &list) {
                    Some(i) => i,
                    None => {
                        media_queries.push(list);
                        media_queries.len() - 1
                    }
                };
                Some(idx)
            }
        };

        let custom_props_only = rule.declarations.is_empty() && !rule.custom_props.is_empty();

        // Classify each selector in the rule's selector list.
        let mut classified_selectors: Vec<(&ComplexSelector, ClassifiedSelector)> = Vec::new();
        for sel in &rule.selectors.0 {
            match classify_selector(sel, allow_combinators, custom_props_only) {
                SelectorCheck::Supported(c) => {
                    for cls in &c.subject {
                        referenced_classes.insert(cls.clone());
                    }
                    for anc in &c.ancestors {
                        for cls in &anc.classes {
                            referenced_classes.insert(cls.clone());
                        }
                    }
                    classified_selectors.push((sel, c));
                }
                SelectorCheck::DocumentRoot => {}
                SelectorCheck::Unsupported(msg) => {
                    let sel_range = sel.span.start as usize..sel.span.end as usize;
                    warnings.push(StyleWarning::sheet_warning(sel_range, msg));
                }
            }
        }

        if classified_selectors.is_empty() || rule.declarations.is_empty() {
            continue;
        }

        // Partition declarations into normal and `!important`.
        let (normal, important): (Vec<Declaration>, Vec<Declaration>) = rule
            .declarations
            .iter()
            .cloned()
            .partition(|d| !d.important);

        // Check if this rule uses any element-scoped custom properties.
        let uses_scoped_vars =
            allow_combinators && decls_reference_any_var(&rule.declarations, &scoped_var_names);

        if uses_scoped_vars {
            for (sel, classified) in classified_selectors {
                for (is_important, decls) in [(false, &normal), (true, &important)] {
                    if decls.is_empty() {
                        continue;
                    }
                    deferred_var_rules.push(DeferredVarRule {
                        ancestors: classified.ancestors.clone(),
                        subject: classified.subject.clone(),
                        subject_pseudo: classified.subject_pseudo,
                        pseudo_element: classified.pseudo_element.clone(),
                        rank: rule.cascade_key(sel, is_important),
                        declarations: decls.clone(),
                        query: query_idx,
                        media: media_idx,
                        span: rule_range.clone(),
                    });
                }
            }
            continue;
        }

        // Group selectors by pseudo-element (`None` vs `Some(pe)`) so we compile
        // declarations once per distinct pseudo-element rather than duplicating
        // diagnostics across comma-separated selectors.
        let mut by_pe: Vec<(
            Option<PseudoElement>,
            Vec<(&ComplexSelector, ClassifiedSelector)>,
        )> = Vec::new();
        for item in classified_selectors {
            let pe = item.1.pseudo_element.clone();
            if let Some((_, group)) = by_pe.iter_mut().find(|(k, _)| *k == pe) {
                group.push(item);
            } else {
                by_pe.push((pe, vec![item]));
            }
        }

        for (pe, group) in by_pe {
            let is_scrollbar_pseudo = matches!(
                pe,
                Some(
                    PseudoElement::WebkitScrollbar
                        | PseudoElement::WebkitScrollbarThumb
                        | PseudoElement::WebkitScrollbarTrack
                )
            );
            for (is_important, decls) in [(false, &normal), (true, &important)] {
                if decls.is_empty() {
                    continue;
                }
                let mut decl_diags = Vec::new();
                let (ops, compiled_trans, compiled_anims) = compile_decls_dual(
                    decls,
                    &root_light,
                    &root_dark,
                    pe.as_ref(),
                    &mut decl_diags,
                );
                push_diags(decl_diags, &mut warnings, &mut errors);

                let trans_items = transition_items_from_compiled(
                    &compiled_trans,
                    rule_range.clone(),
                    &mut warnings,
                );
                let anim_items = animation_items_from_compiled(
                    &compiled_anims,
                    &compiled_kf,
                    rule_range.clone(),
                    &mut warnings,
                );

                if query_idx.is_some() && !trans_items.is_empty() {
                    warnings.push(StyleWarning::sheet_warning(
                        rule_range.clone(),
                        "`transition` inside `@container` is not supported and is ignored",
                    ));
                }
                if query_idx.is_some() && !anim_items.is_empty() {
                    warnings.push(StyleWarning::sheet_warning(
                        rule_range.clone(),
                        "`animation` inside `@container` is not supported and is ignored",
                    ));
                }

                let patch = emit_patch_stmts(&ops, "__nana_patch", is_scrollbar_pseudo);

                for (sel, classified) in &group {
                    let classes = if classified.ancestors.is_empty() {
                        classified.subject.clone()
                    } else {
                        vec![intern_combinator(
                            &mut combinators,
                            &mut referenced_classes,
                            classified.ancestors.clone(),
                            classified.subject.clone(),
                        )]
                    };
                    let rank = rule.cascade_key(sel, is_important);
                    if !patch.is_empty() {
                        rules.push(Rule {
                            classes: classes.clone(),
                            pseudo: classified.subject_pseudo,
                            rank,
                            ops: ops.clone(),
                            patch: patch.clone(),
                            query: query_idx,
                            media: media_idx,
                            is_scrollbar_pseudo,
                        });
                    }
                    if query_idx.is_none() && !trans_items.is_empty() {
                        transitions.push(TransitionRule {
                            classes: classes.clone(),
                            pseudo: classified.subject_pseudo,
                            rank,
                            items: trans_items.clone(),
                        });
                    }
                    if query_idx.is_none() && !anim_items.is_empty() {
                        animations.push(AnimationRule {
                            classes,
                            pseudo: classified.subject_pseudo,
                            rank,
                            items: anim_items.clone(),
                        });
                    }
                }
            }
        }
    }

    Sheet {
        rules,
        transitions,
        animations,
        combinators,
        scoped_var_rules,
        deferred_var_rules,
        referenced_classes,
        queries,
        media_queries,
        font_faces: parsed.font_faces,
        root_light: Arc::new(root_light),
        root_dark: Arc::new(root_dark),
        compiled_kf,
        warnings,
        errors,
    }
}

// ============================================================================
// Rust Token Codegen for Sheet, Queries, Media, Font-Faces, Motion
// ============================================================================

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

const BLOCKS: &[&str] = &["Suspense", "ErrorBoundary"];
const STRUCTURAL_CONTAINERS: &[&str] = &[
    "Block",
    "Virtual",
    "Transition",
    "TransitionGroup",
    "KeepAlive",
    "Teleport",
];

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

    fn items(&self, name: &syn::Ident, runtime: &TokenStream) -> SheetItems {
        self.items_with_hot(name, runtime, Vec::new(), None)
    }

    fn items_with_hot(
        &self,
        name: &syn::Ident,
        runtime: &TokenStream,
        initial_classes: Vec<String>,
        hot_view: Option<&str>,
    ) -> SheetItems {
        let mut classes = initial_classes;
        let mut patches: BTreeMap<&str, usize> = BTreeMap::new();
        let mut queries: BTreeSet<usize> = BTreeSet::new();
        let mut media_emitted: BTreeSet<usize> = BTreeSet::new();
        let mut items = Vec::new();

        let mut ordered: Vec<&Rule> = self.rules.iter().collect();
        ordered.sort_by_key(|rule| rule.rank);

        let mut rules_tokens = Vec::new();
        let mut rule_media_pairs: Vec<TokenStream> = Vec::new();

        for (rule_idx, rule) in ordered.iter().enumerate() {
            let next = patches.len();
            let index = *patches.entry(rule.patch.as_str()).or_insert(next);
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
            if let Some(mq_idx) = rule.media {
                let mq_ident = format_ident!("{name}_MEDIA_{mq_idx}");
                if media_emitted.insert(mq_idx) {
                    let expr_str =
                        emit_media_query_list_rust(&self.media_queries[mq_idx], "__nana_patch");
                    let expr: TokenStream = expr_str
                        .parse()
                        .expect("emitted MediaQueryList parses as Rust expr");
                    items.push(quote! {
                        static #mq_ident: #runtime::view::SheetMediaQuery =
                            #runtime::view::SheetMediaQuery::new(|| {
                                #[allow(unused_imports)]
                                use #runtime::view::__patch as __nana_patch;
                                #expr
                            });
                    });
                }
                rule_media_pairs.push(quote!((#rule_idx, &#mq_ident)));
            }
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            let pseudo = rule.pseudo;
            rules_tokens.push(quote! {
                #runtime::view::SheetRule {
                    classes: &[#(#needs),*],
                    pseudo: #pseudo,
                    patch: &#patch,
                    query: #query,
                }
            });
        }

        let mut ordered_trans: Vec<&TransitionRule> = self.transitions.iter().collect();
        ordered_trans.sort_by_key(|rule| rule.rank);
        let mut transitions = Vec::new();
        for rule in ordered_trans {
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

        let mut sheet_expr = if let Some(view_name) = hot_view {
            quote! {
                #runtime::view::Sheet::with_hot(
                    &[#(#rules_tokens),*],
                    &[#(#transitions),*],
                    &[#(#animations),*],
                    #view_name,
                    &[#(#classes),*],
                )
            }
        } else if animations.is_empty() {
            quote! {
                #runtime::view::Sheet::new(&[#(#rules_tokens),*], &[#(#transitions),*])
            }
        } else {
            quote! {
                #runtime::view::Sheet::with_animations(
                    &[#(#rules_tokens),*],
                    &[#(#transitions),*],
                    &[#(#animations),*],
                )
            }
        };

        if !rule_media_pairs.is_empty() {
            sheet_expr = quote!(#sheet_expr.with_media(&[#(#rule_media_pairs),*]));
        }

        if !self.font_faces.is_empty() {
            let face_exprs: Vec<TokenStream> = self
                .font_faces
                .iter()
                .map(|ff| {
                    emit_font_face_rule_rust(ff, "__nana_patch")
                        .parse()
                        .expect("emitted FontFaceRule parses as Rust expr")
                })
                .collect();
            sheet_expr = quote! {
                #sheet_expr.with_font_faces(|| {
                    #[allow(unused_imports)]
                    use #runtime::view::__patch as __nana_patch;
                    ::std::vec![#(#face_exprs),*]
                })
            };
        }

        items.push(quote! {
            static #name: #runtime::view::Sheet = #sheet_expr;
        });

        SheetItems { classes, items }
    }
}

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

// ============================================================================
// Template Tree Matching (Combinators, Ancestor Pseudos, Scoped Vars, Inline)
// ============================================================================

struct AncestorFrame {
    fixed: Vec<String>,
    conditional: Vec<(String, Expr)>,
    /// True if this ancestor is a real `El` that supports `.track_interactive_state`.
    can_track_interactive: bool,
    /// Allocated `__anc_N` handle if any descendant matches an interactive pseudo on this frame.
    interactive_handle: Option<syn::Ident>,
    /// Accumulated interactive pseudo mask observed on this ancestor.
    tracked_mask: u8,
}

impl Default for AncestorFrame {
    fn default() -> Self {
        Self {
            fixed: Vec::new(),
            conditional: Vec::new(),
            can_track_interactive: false,
            interactive_handle: None,
            tracked_mask: 0,
        }
    }
}

#[derive(Clone)]
enum MatchTerm {
    Expr(Expr),
    /// `(ancestor_frame_index, pseudo_mask)`
    AncestorPseudo(usize, u8),
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

fn match_compound_fixed_only(required: &[String], fixed: &[String]) -> bool {
    required
        .iter()
        .all(|class| fixed.iter().any(|known| known == class))
}

fn match_chain_fixed_only(chain: &[AncestorStep], ancestors: &[AncestorFrame]) -> bool {
    if chain.is_empty() {
        return true;
    }
    let (step, rest_chain) = (&chain[chain.len() - 1], &chain[..chain.len() - 1]);
    if step.pseudo != 0 {
        return false;
    }
    match step.combinator {
        Combinator::Child => {
            let Some((parent, rest_ancestors)) = ancestors.split_last() else {
                return false;
            };
            match_compound_fixed_only(&step.classes, &parent.fixed)
                && match_chain_fixed_only(rest_chain, rest_ancestors)
        }
        Combinator::Descendant => {
            for idx in (1..=ancestors.len()).rev() {
                if match_compound_fixed_only(&step.classes, &ancestors[idx - 1].fixed)
                    && match_chain_fixed_only(rest_chain, &ancestors[..idx - 1])
                {
                    return true;
                }
            }
            false
        }
        Combinator::NextSibling | Combinator::SubsequentSibling => false,
    }
}

fn match_chain(chain: &[AncestorStep], ancestors: &[AncestorFrame]) -> Vec<Vec<MatchTerm>> {
    if chain.is_empty() {
        return vec![Vec::new()];
    }
    let (step, rest_chain) = (&chain[chain.len() - 1], &chain[..chain.len() - 1]);
    match step.combinator {
        Combinator::Child => {
            let Some((parent, rest_ancestors)) = ancestors.split_last() else {
                return Vec::new();
            };
            if step.pseudo != 0 && !parent.can_track_interactive {
                return Vec::new();
            }
            let Some(parent_conds) =
                match_compound(&step.classes, &parent.fixed, &parent.conditional)
            else {
                return Vec::new();
            };
            let parent_idx = ancestors.len() - 1;
            match_chain(rest_chain, rest_ancestors)
                .into_iter()
                .map(|mut clause| {
                    clause.extend(parent_conds.iter().cloned().map(MatchTerm::Expr));
                    if step.pseudo != 0 {
                        clause.push(MatchTerm::AncestorPseudo(parent_idx, step.pseudo));
                    }
                    clause
                })
                .collect()
        }
        Combinator::Descendant => {
            let mut clauses = Vec::new();
            for idx in (1..=ancestors.len()).rev() {
                let candidate_idx = idx - 1;
                let candidate = &ancestors[candidate_idx];
                if step.pseudo != 0 && !candidate.can_track_interactive {
                    continue;
                }
                let rest_ancestors = &ancestors[..candidate_idx];
                let Some(anc_conds) =
                    match_compound(&step.classes, &candidate.fixed, &candidate.conditional)
                else {
                    continue;
                };
                for mut clause in match_chain(rest_chain, rest_ancestors) {
                    clause.extend(anc_conds.iter().cloned().map(MatchTerm::Expr));
                    if step.pseudo != 0 {
                        clause.push(MatchTerm::AncestorPseudo(candidate_idx, step.pseudo));
                    }
                    if clause.is_empty() {
                        return vec![Vec::new()];
                    }
                    clauses.push(clause);
                }
            }
            clauses
        }
        Combinator::NextSibling | Combinator::SubsequentSibling => Vec::new(),
    }
}

fn wrap_condition_source(expr: &Expr, runtime: &TokenStream) -> TokenStream {
    match expr {
        Expr::Path(_) | Expr::Closure(_) | Expr::Lit(_) | Expr::Field(_) => {
            quote!(#runtime::view::IntoProp::<bool>::into_source(#expr))
        }
        other => quote!(#runtime::view::IntoProp::<bool>::into_source(move || (#other))),
    }
}

/// Format a conditional `class:name=...` expression cleanly without double-wrapping closures.
fn class_condition_tokens(expr: &Expr) -> TokenStream {
    match expr {
        Expr::Path(_) | Expr::Closure(_) | Expr::Lit(_) | Expr::Field(_) => quote!(#expr),
        other => quote!(move || #other),
    }
}

#[derive(Clone)]
enum ResolvedTerm {
    Expr(Expr),
    AncestorPseudo(syn::Ident, u8),
}

fn combine_dnf_condition(dnf: Vec<Vec<ResolvedTerm>>, runtime: &TokenStream) -> TokenStream {
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
            for term in clause {
                match term {
                    ResolvedTerm::Expr(expr) => {
                        let idx = index_of(expr);
                        let ident = format_ident!("__nana_c{idx}");
                        terms.push(quote!(#ident.get()));
                    }
                    ResolvedTerm::AncestorPseudo(handle, mask) => {
                        terms.push(quote!(#handle.matches(#mask)));
                    }
                }
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
    if unique.is_empty() {
        quote!(move || (#body))
    } else {
        quote!({
            #(#bindings)*
            move || (#body)
        })
    }
}

enum SynthAttachment {
    Fixed(String, Span),
    AncestorPseudo(String, syn::Ident, u8, Span),
    Conditional(String, TokenStream, Span),
}

struct TemplateWalk<'a> {
    sheet: &'a mut Sheet,
    name: syn::Ident,
    runtime: &'a TokenStream,
    attach_all: bool,
    ancestors: Vec<AncestorFrame>,
    var_stack: Vec<(Arc<VarScope>, Arc<VarScope>)>,
    /// Deduplicated inline patches emitted for `style="..."` attributes:
    /// `patch_stmts -> patch_ident`.
    inline_patches: BTreeMap<String, syn::Ident>,
    inline_items: Vec<TokenStream>,
    /// Deduplicated specialized deferred-var rules:
    /// `(deferred_rule_idx, patch_stmts) -> synth_class`.
    deferred_specializations: BTreeMap<(usize, String), String>,
    /// Track which deferred rules matched at least one template element.
    matched_deferred: BTreeSet<usize>,
    /// Next `__anc_N` counter for ancestor interactive state handles.
    next_anc_id: usize,
    /// Pending element injections (resolved after sheet classes are finalized? No —
    /// we collect all deferred rules during pass 1, finalize sheet items, or allocate
    /// class indices directly? Let's do clean two-step: walk collects `sheet.rules`
    /// extensions & attaches class names, OR records class names in a mutable `Vec<String>`
    /// that `Sheet::items_with_hot` uses!).
    classes: Vec<String>,
    sheet_classes: BTreeSet<String>,
    used_sheet: bool,
    attached_font_sheet: bool,
    warnings: Vec<StyleWarning>,
    errors: Vec<StyleWarning>,
}

impl<'a> TemplateWalk<'a> {
    fn new(
        sheet: &'a mut Sheet,
        initial_classes: Vec<String>,
        name: syn::Ident,
        runtime: &'a TokenStream,
        attach_all: bool,
    ) -> Self {
        let mut sheet_classes = BTreeSet::new();
        for rule in &sheet.rules {
            sheet_classes.extend(rule.classes.iter().cloned());
        }
        for rule in &sheet.transitions {
            sheet_classes.extend(rule.classes.iter().cloned());
        }
        for rule in &sheet.animations {
            sheet_classes.extend(rule.classes.iter().cloned());
        }
        let root_light = Arc::clone(&sheet.root_light);
        let root_dark = Arc::clone(&sheet.root_dark);
        // Seed `classes` with all existing rule/transition/animation classes so
        // indices are stable and known before walking.
        let mut classes = initial_classes;
        for c in &sheet_classes {
            Sheet::class_index(&mut classes, c);
        }
        Self {
            sheet,
            name,
            runtime,
            attach_all,
            ancestors: Vec::new(),
            var_stack: vec![(root_light, root_dark)],
            inline_patches: BTreeMap::new(),
            inline_items: Vec::new(),
            deferred_specializations: BTreeMap::new(),
            matched_deferred: BTreeSet::new(),
            next_anc_id: 0,
            classes,
            sheet_classes,
            used_sheet: false,
            attached_font_sheet: false,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn warn(&mut self, span: Span, message: String) {
        self.warnings.push(StyleWarning {
            at: StyleAt::Template(span),
            severity: StyleSeverity::Warning,
            message,
        });
    }

    fn alloc_ancestor_handle(&mut self, anc_idx: usize, mask: u8) -> syn::Ident {
        let frame = &mut self.ancestors[anc_idx];
        frame.tracked_mask |= mask;
        if let Some(existing) = &frame.interactive_handle {
            return existing.clone();
        }
        let ident = format_ident!("__anc_{}", self.next_anc_id);
        self.next_anc_id += 1;
        frame.interactive_handle = Some(ident.clone());
        ident
    }

    fn resolve_match_dnf(
        &mut self,
        class_name: String,
        dnf: Vec<Vec<MatchTerm>>,
        span: Span,
    ) -> SynthAttachment {
        if dnf.iter().any(|clause| clause.is_empty()) {
            return SynthAttachment::Fixed(class_name, span);
        }
        // Fast path: single clause with a single `AncestorPseudo` term -> `.ancestor_pseudo(class, handle, mask)`.
        if dnf.len() == 1
            && dnf[0].len() == 1
            && let MatchTerm::AncestorPseudo(anc_idx, mask) = dnf[0][0]
        {
            let handle = self.alloc_ancestor_handle(anc_idx, mask);
            return SynthAttachment::AncestorPseudo(class_name, handle, mask, span);
        }
        let resolved: Vec<Vec<ResolvedTerm>> = dnf
            .into_iter()
            .map(|clause| {
                clause
                    .into_iter()
                    .map(|term| match term {
                        MatchTerm::Expr(e) => ResolvedTerm::Expr(e),
                        MatchTerm::AncestorPseudo(anc_idx, mask) => {
                            let handle = self.alloc_ancestor_handle(anc_idx, mask);
                            ResolvedTerm::AncestorPseudo(handle, mask)
                        }
                    })
                    .collect()
            })
            .collect();
        let expr_tokens = combine_dnf_condition(resolved, self.runtime);
        SynthAttachment::Conditional(class_name, expr_tokens, span)
    }

    fn intern_inline_patch(&mut self, patch_stmts: String) -> syn::Ident {
        if let Some(existing) = self.inline_patches.get(&patch_stmts) {
            return existing.clone();
        }
        let idx = self.inline_patches.len();
        let ident = format_ident!("__NANA_INLINE_PATCH_{idx}");
        let apply = patch_tokens(&patch_stmts, self.runtime);
        let runtime = self.runtime;
        self.inline_items.push(quote! {
            static #ident: #runtime::view::StylePatch = #runtime::view::StylePatch::new(#apply);
        });
        self.inline_patches.insert(patch_stmts, ident.clone());
        ident
    }

    fn build_element_var_scopes(
        &self,
        fixed_names: &[String],
        inline_css: Option<&str>,
    ) -> (Arc<VarScope>, Arc<VarScope>) {
        let (parent_light, parent_dark) = self.var_stack.last().expect("var_stack non-empty");

        let mut rule_props: Vec<(&CustomPropDecl, CascadeKey)> = Vec::new();
        for vr in &self.sheet.scoped_var_rules {
            if !match_compound_fixed_only(&vr.subject, fixed_names) {
                continue;
            }
            if !match_chain_fixed_only(&vr.ancestors, &self.ancestors) {
                continue;
            }
            for cp in &vr.custom_props {
                let key = CascadeKey::new(vr.layer, cp.important, vr.specificity, vr.source_order);
                rule_props.push((cp, key));
            }
        }

        let inline_custom_props: Vec<CustomPropDecl> = inline_css
            .map(|css| {
                compile_inline_css(css, parent_light, &CompileContext::default()).custom_props
            })
            .unwrap_or_default();

        if rule_props.is_empty() && inline_custom_props.is_empty() {
            return (Arc::clone(parent_light), Arc::clone(parent_dark));
        }

        let make_child = |parent: &Arc<VarScope>| {
            let mut scope = VarScope::with_parent(Arc::clone(parent));
            for (cp, key) in &rule_props {
                scope.set_with_cascade(cp.name.clone(), cp.value.clone(), key.to_var_key());
            }
            if !inline_custom_props.is_empty() {
                let mut inline_scope = VarScope::with_parent(Arc::new(scope));
                for cp in &inline_custom_props {
                    inline_scope.set(cp.name.clone(), cp.value.clone());
                }
                Arc::new(inline_scope)
            } else {
                Arc::new(scope)
            }
        };

        (make_child(parent_light), make_child(parent_dark))
    }

    fn nodes(&mut self, nodes: &mut [Node]) {
        for node in nodes {
            if let Node::Element(element) = node {
                self.element(element);
            }
        }
    }

    fn element(&mut self, element: &mut Element) {
        let tag = element.tag();
        let at_element = element.name.span();
        let is_real_builtin = crate::is_builtin(&tag) && !BLOCKS.contains(&tag.as_str());
        let is_el = is_real_builtin && !STRUCTURAL_CONTAINERS.contains(&tag.as_str());
        let is_span = element.module.is_empty() && element.name == "Span";

        let mut fixed: Vec<(String, Span)> = Vec::new();
        let mut conditional: Vec<(String, Expr, Span)> = Vec::new();
        let mut inline_style: Option<(String, Span)> = None;
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
                // Fix `class:` bugs: accept BOTH `AttrValue::Expr` and `AttrValue::Lit` (e.g. `class:active=true`).
                (AttrName::Directive(directive, span), AttrValue::Expr(condition))
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    conditional.push((class, condition.clone(), *span));
                }
                (AttrName::Directive(directive, span), AttrValue::Lit(condition))
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    conditional.push((class, condition.clone(), *span));
                }
                (AttrName::Directive(directive, span), AttrValue::None)
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    let cond: Expr = syn::parse_quote_spanned!(*span=> true);
                    conditional.push((class, cond, *span));
                }
                // Inline `style="..."` on built-in elements (preserving `<Span style={RichSpanStyle}>`).
                (
                    AttrName::Plain(name),
                    AttrValue::Lit(Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(text),
                        ..
                    })),
                ) if name == "style" && is_el && !is_span => match &mut inline_style {
                    Some((existing, _)) => {
                        existing.push(';');
                        existing.push_str(&text.value());
                    }
                    None => inline_style = Some((text.value(), name.span())),
                },
                _ => kept.push(attr),
            }
        }

        let fixed_names: Vec<String> = fixed.iter().map(|(c, _)| c.clone()).collect();
        let cond_pairs: Vec<(String, Expr)> = conditional
            .iter()
            .map(|(c, e, _)| (c.clone(), e.clone()))
            .collect();

        // Compute this element's hierarchical `VarScope` (light & dark).
        let (var_light, var_dark) = self.build_element_var_scopes(
            &fixed_names,
            inline_style.as_ref().map(|(css, _)| css.as_str()),
        );

        // Compile inline `style="..."` against this element's resolved `VarScope`.
        let mut inline_injected: Vec<Attr> = Vec::new();
        if let Some((css_text, style_span)) = inline_style {
            let (ops, compiled_trans, diags) =
                compile_inline_dual(&css_text, &var_light, &var_dark);
            push_template_diags(style_span, diags, &mut self.warnings, &mut self.errors);
            if !ops.is_empty() {
                let stmts = emit_patch_stmts(&ops, "__nana_patch", false);
                let patch_ident = self.intern_inline_patch(stmts);
                inline_injected.push(Attr {
                    name: AttrName::Directive("inline_patch".into(), style_span),
                    value: AttrValue::Verbatim(quote!(&#patch_ident)),
                });
            }
            if !compiled_trans.is_empty() {
                let mut tr_warnings = Vec::new();
                let items = transition_items_from_compiled(
                    &compiled_trans,
                    0..css_text.len(),
                    &mut tr_warnings,
                );
                for w in tr_warnings {
                    self.warn(style_span, w.message);
                }
                if !items.is_empty() {
                    let anim_tokens = animate_tokens(&items, self.runtime);
                    inline_injected.push(Attr {
                        name: AttrName::Directive("inline_animate".into(), style_span),
                        value: AttrValue::Verbatim(anim_tokens),
                    });
                }
            }
        }

        // Match static/interactive combinator targets against ancestors + self.
        let mut synth_attachments: Vec<SynthAttachment> = Vec::new();
        if (!fixed_names.is_empty() || !cond_pairs.is_empty()) && is_real_builtin {
            let combinators = self.sheet.combinators.clone();
            for target in combinators {
                let Some(subj_conds) = match_compound(&target.subject, &fixed_names, &cond_pairs)
                else {
                    continue;
                };
                let anc_dnf = match_chain(&target.ancestors, &self.ancestors);
                if anc_dnf.is_empty() {
                    continue;
                }
                let combined: Vec<Vec<MatchTerm>> = anc_dnf
                    .into_iter()
                    .map(|mut clause| {
                        clause.extend(subj_conds.iter().cloned().map(MatchTerm::Expr));
                        clause
                    })
                    .collect();
                synth_attachments.push(self.resolve_match_dnf(
                    target.synth_class,
                    combined,
                    at_element,
                ));
            }

            // Match deferred element-scoped-var rules and specialize them against `(var_light, var_dark)`.
            let deferred_len = self.sheet.deferred_var_rules.len();
            for def_idx in 0..deferred_len {
                let def = self.sheet.deferred_var_rules[def_idx].clone();
                let Some(subj_conds) = match_compound(&def.subject, &fixed_names, &cond_pairs)
                else {
                    continue;
                };
                let anc_dnf = match_chain(&def.ancestors, &self.ancestors);
                if anc_dnf.is_empty() {
                    continue;
                }
                self.matched_deferred.insert(def_idx);
                let is_scrollbar_pseudo = matches!(
                    def.pseudo_element,
                    Some(
                        PseudoElement::WebkitScrollbar
                            | PseudoElement::WebkitScrollbarThumb
                            | PseudoElement::WebkitScrollbarTrack
                    )
                );
                let mut decl_diags = Vec::new();
                let (ops, compiled_trans, compiled_anims) = compile_decls_dual(
                    &def.declarations,
                    &var_light,
                    &var_dark,
                    def.pseudo_element.as_ref(),
                    &mut decl_diags,
                );
                push_diags(decl_diags, &mut self.warnings, &mut self.errors);

                let trans_items = transition_items_from_compiled(
                    &compiled_trans,
                    def.span.clone(),
                    &mut self.warnings,
                );
                let anim_items = animation_items_from_compiled(
                    &compiled_anims,
                    &self.sheet.compiled_kf,
                    def.span.clone(),
                    &mut self.warnings,
                );
                let patch = emit_patch_stmts(&ops, "__nana_patch", is_scrollbar_pseudo);
                if patch.is_empty() && trans_items.is_empty() && anim_items.is_empty() {
                    continue;
                }

                let spec_key = (def_idx, patch.clone());
                let synth_class =
                    if let Some(existing) = self.deferred_specializations.get(&spec_key) {
                        existing.clone()
                    } else {
                        let synth = format!("__nana_var_{}", self.deferred_specializations.len());
                        self.deferred_specializations
                            .insert(spec_key, synth.clone());
                        Sheet::class_index(&mut self.classes, &synth);
                        self.sheet_classes.insert(synth.clone());
                        if !patch.is_empty() {
                            self.sheet.rules.push(Rule {
                                classes: vec![synth.clone()],
                                pseudo: def.subject_pseudo,
                                rank: def.rank,
                                ops,
                                patch,
                                query: def.query,
                                media: def.media,
                                is_scrollbar_pseudo,
                            });
                        }
                        if def.query.is_none() && !trans_items.is_empty() {
                            self.sheet.transitions.push(TransitionRule {
                                classes: vec![synth.clone()],
                                pseudo: def.subject_pseudo,
                                rank: def.rank,
                                items: trans_items,
                            });
                        }
                        if def.query.is_none() && !anim_items.is_empty() {
                            self.sheet.animations.push(AnimationRule {
                                classes: vec![synth.clone()],
                                pseudo: def.subject_pseudo,
                                rank: def.rank,
                                items: anim_items,
                            });
                        }
                        synth
                    };

                let combined: Vec<Vec<MatchTerm>> = anc_dnf
                    .into_iter()
                    .map(|mut clause| {
                        clause.extend(subj_conds.iter().cloned().map(MatchTerm::Expr));
                        clause
                    })
                    .collect();
                synth_attachments.push(self.resolve_match_dnf(synth_class, combined, at_element));
            }
        }

        // Push ancestor & var frames, recurse into children and slot views, then pop.
        self.ancestors.push(AncestorFrame {
            fixed: fixed_names,
            conditional: cond_pairs,
            can_track_interactive: is_el && !is_span,
            interactive_handle: None,
            tracked_mask: 0,
        });
        self.var_stack.push((var_light, var_dark));

        self.nodes(&mut element.children);
        for attr in &mut kept {
            if let AttrValue::View(slot) = &mut attr.value {
                self.ancestors.push(AncestorFrame::default());
                self.nodes(slot);
                self.ancestors.pop();
            }
        }

        self.var_stack.pop();
        let popped_frame = self.ancestors.pop().expect("balanced ancestor push/pop");

        element.attrs = kept;

        let mut injected = Vec::new();

        // Attach `.sheet(&__NANA_SHEET)` on the first real built-in element when
        // the stylesheet declares `@font-face` rules.
        if is_el && !is_span && !self.attached_font_sheet && !self.sheet.font_faces.is_empty() {
            let name = &self.name;
            injected.push(Attr {
                name: AttrName::Directive("sheet".into(), at_element),
                value: AttrValue::Verbatim(quote!(&#name)),
            });
            self.attached_font_sheet = true;
            self.used_sheet = true;
        }

        // Attach `.track_interactive_state(__anc_N, mask)` if any descendant matched an ancestor pseudo here.
        if let Some(handle) = popped_frame.interactive_handle
            && popped_frame.tracked_mask != 0
        {
            let mask = popped_frame.tracked_mask;
            injected.push(Attr {
                name: AttrName::Directive("track_interactive".into(), at_element),
                value: AttrValue::Verbatim(quote!(#handle, #mask)),
            });
        }

        // Attach inline `style="..."` patches/transitions.
        injected.extend(inline_injected);

        let has_class_work =
            !fixed.is_empty() || !conditional.is_empty() || !synth_attachments.is_empty();

        if has_class_work {
            if !is_real_builtin {
                self.warn(
                    at_element,
                    format!("`<{tag}>` takes no class: style the elements inside it"),
                );
            } else {
                let dynamic_count = conditional.len()
                    + synth_attachments
                        .iter()
                        .filter(|a| !matches!(a, SynthAttachment::Fixed(..)))
                        .count();
                if dynamic_count > MAX_CONDITIONAL_CLASSES {
                    self.warn(
                        at_element,
                        format!(
                            "`<{tag}>` has more than {MAX_CONDITIONAL_CLASSES} conditional classes"
                        ),
                    );
                } else {
                    let all_fixed: Vec<String> = fixed
                        .iter()
                        .map(|(name, _)| name.clone())
                        .chain(synth_attachments.iter().filter_map(|a| match a {
                            SynthAttachment::Fixed(name, _) => Some(name.clone()),
                            _ => None,
                        }))
                        .collect();
                    let all_conditional: Vec<String> = conditional
                        .iter()
                        .map(|(name, _, _)| name.clone())
                        .chain(synth_attachments.iter().filter_map(|a| match a {
                            SynthAttachment::AncestorPseudo(name, ..)
                            | SynthAttachment::Conditional(name, ..) => Some(name.clone()),
                            SynthAttachment::Fixed(..) => None,
                        }))
                        .collect();
                    let holds = |classes: &[String], conditional_too: bool| {
                        classes.iter().all(|class| {
                            all_fixed.iter().any(|name| name == class)
                                || (conditional_too
                                    && all_conditional.iter().any(|name| name == class))
                        })
                    };
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
                                "`<{tag}>`: its classes take `@container` rules on more than one \
                                 container or axis; while such rules apply together, none of them \
                                 does"
                            ),
                        ),
                        Err(ContainerPlanUnsupported::TooManyBreakpoints) => self.warn(
                            at_element,
                            format!(
                                "`<{tag}>`: its classes take `@container` rules with more than \
                                 {MAX_CONTAINER_BREAKPOINTS} breakpoints; while they all apply, \
                                 none of them does"
                            ),
                        ),
                        Ok(_) => {}
                    }

                    let runtime = self.runtime;
                    let name = self.name.clone();
                    let index_of = |classes: &[String], class: &str| {
                        classes
                            .iter()
                            .position(|known| known == class)
                            .map(|at| at as u16)
                    };

                    let mut added_sheet_attr = false;
                    for (class, span) in fixed {
                        let known_in_sheet = self.sheet_classes.contains(&class)
                            || self.sheet.referenced_classes.contains(&class);
                        match index_of(&self.classes, &class) {
                            Some(at) if known_in_sheet || self.attach_all => {
                                injected.push(Attr {
                                    name: AttrName::Directive("class".into(), span),
                                    value: AttrValue::Verbatim(
                                        quote!(#runtime::view::Class::new(&#name, #at)),
                                    ),
                                });
                                added_sheet_attr = true;
                                if !known_in_sheet {
                                    self.warn(
                                        span,
                                        format!(
                                            "`<{tag}>`: no rule in `<style>` uses class `{class}`"
                                        ),
                                    );
                                }
                            }
                            Some(_) | None if self.sheet.referenced_classes.contains(&class) => {}
                            Some(_) | None => self.warn(
                                span,
                                format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                            ),
                        }
                    }

                    for (class, condition, span) in conditional {
                        let known_in_sheet = self.sheet_classes.contains(&class)
                            || self.sheet.referenced_classes.contains(&class);
                        let Some(at) = index_of(&self.classes, &class)
                            .filter(|_| known_in_sheet || self.attach_all)
                        else {
                            if !known_in_sheet {
                                self.warn(
                                    span,
                                    format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                                );
                            }
                            continue;
                        };
                        if !known_in_sheet {
                            self.warn(
                                span,
                                format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                            );
                        }
                        let cond_tokens = class_condition_tokens(&condition);
                        injected.push(Attr {
                            name: AttrName::Directive("class_when".into(), span),
                            value: AttrValue::Verbatim(
                                quote!(#runtime::view::Class::new(&#name, #at), #cond_tokens),
                            ),
                        });
                        added_sheet_attr = true;
                    }

                    for attachment in synth_attachments {
                        match attachment {
                            SynthAttachment::Fixed(class, span) => {
                                if let Some(at) = index_of(&self.classes, &class) {
                                    injected.push(Attr {
                                        name: AttrName::Directive("class".into(), span),
                                        value: AttrValue::Verbatim(
                                            quote!(#runtime::view::Class::new(&#name, #at)),
                                        ),
                                    });
                                    added_sheet_attr = true;
                                }
                            }
                            SynthAttachment::AncestorPseudo(class, handle, mask, span) => {
                                if let Some(at) = index_of(&self.classes, &class) {
                                    injected.push(Attr {
                                        name: AttrName::Directive("ancestor_pseudo".into(), span),
                                        value: AttrValue::Verbatim(
                                            quote!(#runtime::view::Class::new(&#name, #at), #handle, #mask),
                                        ),
                                    });
                                    added_sheet_attr = true;
                                }
                            }
                            SynthAttachment::Conditional(class, condition, span) => {
                                if let Some(at) = index_of(&self.classes, &class) {
                                    injected.push(Attr {
                                        name: AttrName::Directive("class_when".into(), span),
                                        value: AttrValue::Verbatim(
                                            quote!(#runtime::view::Class::new(&#name, #at), #condition),
                                        ),
                                    });
                                    added_sheet_attr = true;
                                }
                            }
                        }
                    }

                    self.used_sheet |= added_sheet_attr;
                }
            }
        }

        if !injected.is_empty() {
            injected.append(&mut element.attrs);
            element.attrs = injected;
        }
    }

    /// Validate any deferred var-dependent rules that did not match any template
    /// element so invalid properties/values are still surfaced (ignoring only
    /// `MissingVariable` since the variable may be defined on an unmounted scope).
    fn validate_unmatched_deferred(&mut self) {
        let len = self.sheet.deferred_var_rules.len();
        for idx in 0..len {
            if self.matched_deferred.contains(&idx) {
                continue;
            }
            let def = self.sheet.deferred_var_rules[idx].clone();
            let mut diags = Vec::new();
            let _ = compile_decls_dual(
                &def.declarations,
                &self.sheet.root_light,
                &self.sheet.root_dark,
                def.pseudo_element.as_ref(),
                &mut diags,
            );
            diags.retain(|d| d.code != DiagCode::MissingVariable);
            push_diags(diags, &mut self.warnings, &mut self.errors);
        }
    }
}

// ============================================================================
// Hot Reload Helpers
// ============================================================================

fn collect_template_classes(nodes: &[Node], out: &mut BTreeSet<String>) {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        for attr in &element.attrs {
            match (&attr.name, &attr.value) {
                (AttrName::Plain(name), AttrValue::Lit(Expr::Lit(literal))) if name == "class" => {
                    if let Lit::Str(text) = &literal.lit {
                        for class in text.value().split_whitespace() {
                            out.insert(class.to_owned());
                        }
                    }
                }
                (
                    AttrName::Directive(directive, _),
                    AttrValue::Expr(_) | AttrValue::Lit(_) | AttrValue::None,
                ) if directive.starts_with("class:") => {
                    out.insert(directive["class:".len()..].to_owned());
                }
                _ => {}
            }
            if let AttrValue::View(slot) = &attr.value {
                collect_template_classes(slot, out);
            }
        }
        collect_template_classes(&element.children, out);
    }
}

fn hot_query(query: &ContainerQuery) -> HotSheetQuery {
    let axis = match query.axis {
        ContainerAxis::Inline => HotSheetAxis::Inline,
        ContainerAxis::Block => HotSheetAxis::Block,
        ContainerAxis::Width => HotSheetAxis::Width,
        ContainerAxis::Height => HotSheetAxis::Height,
    };
    HotSheetQuery {
        name: query.name.clone(),
        axis,
        intervals: query
            .intervals
            .iter()
            .map(|interval| (interval.lo, interval.hi))
            .collect(),
    }
}

fn hot_style_shape(sheet: &Sheet, runtime: &TokenStream) -> TokenStream {
    let combinators = sheet.combinators.iter().map(|target| {
        let synth = &target.synth_class;
        let subject = &target.subject;
        let ancestors = target.ancestors.iter().map(|step| {
            let kind = match step.combinator {
                Combinator::Child => ">",
                Combinator::Descendant => " ",
                Combinator::NextSibling => "+",
                Combinator::SubsequentSibling => "~",
            };
            let classes = &step.classes;
            let pseudo = step.pseudo;
            quote!((#kind, #pseudo, [#(#classes),*]))
        });
        quote!((#synth, [#(#ancestors),*], [#(#subject),*]);)
    });
    let mut ordered_trans: Vec<&TransitionRule> = sheet.transitions.iter().collect();
    ordered_trans.sort_by_key(|rule| rule.rank);
    let transitions = ordered_trans.iter().map(|rule| {
        let classes = &rule.classes;
        let pseudo = rule.pseudo;
        let animate = animate_tokens(&rule.items, runtime);
        quote!((#pseudo, [#(#classes),*], #animate);)
    });
    let mut ordered_anims: Vec<&AnimationRule> = sheet.animations.iter().collect();
    ordered_anims.sort_by_key(|rule| rule.rank);
    let animations = ordered_anims.iter().map(|rule| {
        let classes = &rule.classes;
        let pseudo = rule.pseudo;
        let entries = rule.items.iter().map(|anim| {
            let stops = anim
                .stops
                .iter()
                .map(|stop| keyframe_stop_tokens(stop, runtime));
            let item = animation_item_tokens(anim, quote!(&[#(#stops),*]), runtime);
            quote!(#item)
        });
        quote!((#pseudo, [#(#classes),*], [#(#entries),*]);)
    });
    let interactive_keys: BTreeSet<(Vec<String>, u8)> = sheet
        .rules
        .iter()
        .filter(|rule| rule.pseudo != 0)
        .map(|rule| (rule.classes.clone(), rule.pseudo))
        .collect();
    let pseudos = interactive_keys
        .iter()
        .map(|(classes, pseudo)| quote!((#pseudo, [#(#classes),*]);));
    let media_count = sheet.media_queries.len();
    let font_count = sheet.font_faces.len();
    quote! {
        {
            #(#combinators)*
            #(#transitions)*
            #(#animations)*
            #(#pseudos)*
            (#media_count, #font_count);
        }
    }
}

fn build_hot_style_sheet(sheet: &Sheet) -> HotStyleSheet {
    let mut ordered_rules: Vec<&Rule> = sheet.rules.iter().collect();
    ordered_rules.sort_by_key(|rule| rule.rank);

    // Coalesce `::-webkit-scrollbar*` rules targeting the same `(classes, pseudo, query)`
    // so `HotStylePatch::from_ops` preserves all merged scrollbar fields.
    let mut merged_scrollbars: BTreeMap<(Vec<String>, u8, Option<usize>), ScrollbarSkin> =
        BTreeMap::new();
    let mut out_rules: Vec<HotStyleRule> = Vec::new();
    for rule in ordered_rules {
        let mut ops = rule.ops.clone();
        if rule.is_scrollbar_pseudo {
            let key = (rule.classes.clone(), rule.pseudo, rule.query);
            let entry = merged_scrollbars.entry(key).or_default();
            for op in &mut ops {
                if let StyleOp::Scrollbar(Some(skin)) = op {
                    merge_scrollbar_skin(entry, skin);
                    *skin = *entry;
                }
            }
        }
        out_rules.push(HotStyleRule {
            classes: rule.classes.clone(),
            pseudo: rule.pseudo,
            patch: HotStylePatch::from_ops(ops),
            query: rule.query.map(|idx| hot_query(&sheet.queries[idx])),
        });
    }
    HotStyleSheet { rules: out_rules }
}

// ============================================================================
// Public Entry Points
// ============================================================================

/// Compile `css` against `nodes` for development hot reload.
pub fn compile_styles_hot(
    css: &str,
    nodes: &mut [Node],
    runtime: &TokenStream,
    view_name: &str,
) -> CompiledHotStyles {
    let mut inline_scoped = BTreeSet::new();
    collect_inline_scoped_var_names(nodes, &mut inline_scoped);
    let mut sheet = parse_sheet(css, true, inline_scoped);
    let mut template_classes = BTreeSet::new();
    collect_template_classes(nodes, &mut template_classes);
    let initial_classes: Vec<String> = template_classes.into_iter().collect();
    let name = format_ident!("__NANA_SHEET");

    let (classes, used_sheet, inline_items, walk_warnings, walk_errors) = {
        let mut walk = TemplateWalk::new(&mut sheet, initial_classes, name.clone(), runtime, true);
        walk.nodes(nodes);
        walk.validate_unmatched_deferred();
        (
            walk.classes,
            walk.used_sheet,
            walk.inline_items,
            walk.warnings,
            walk.errors,
        )
    };

    let SheetItems {
        items: sheet_items, ..
    } = sheet.items_with_hot(&name, runtime, classes, Some(view_name));

    let mut warnings = sheet.warnings.clone();
    warnings.extend(walk_warnings);
    let mut errors = sheet.errors.clone();
    errors.extend(walk_errors);

    let hot_sheet = build_hot_style_sheet(&sheet);
    let style_shape = hot_style_shape(&sheet, runtime);

    let mut items = inline_items;
    if used_sheet {
        items.extend(sheet_items);
    }

    CompiledHotStyles {
        items,
        warnings,
        errors,
        hot_sheet,
        style_shape,
    }
}

/// Compile `css` against the elements of `nodes`: each element's `class="…"`,
/// `class:name="…"`, and inline `style="…"` become `.class(…)`, `.class_when(…)`,
/// `.ancestor_pseudo(…)`, `.track_interactive_state(…)`, `.inline_patch(…)`, and
/// `.sheet(…)` on the view's elements.
pub fn compile_styles(css: &str, nodes: &mut [Node], runtime: &TokenStream) -> CompiledStyles {
    let mut inline_scoped = BTreeSet::new();
    collect_inline_scoped_var_names(nodes, &mut inline_scoped);
    let mut sheet = parse_sheet(css, true, inline_scoped);
    let name = format_ident!("__NANA_SHEET");

    let (classes, used_sheet, inline_items, walk_warnings, walk_errors) = {
        let mut walk = TemplateWalk::new(&mut sheet, Vec::new(), name.clone(), runtime, false);
        walk.nodes(nodes);
        walk.validate_unmatched_deferred();
        (
            walk.classes,
            walk.used_sheet,
            walk.inline_items,
            walk.warnings,
            walk.errors,
        )
    };

    let SheetItems {
        items: sheet_items, ..
    } = sheet.items_with_hot(&name, runtime, classes, None);

    let mut warnings = sheet.warnings.clone();
    warnings.extend(walk_warnings);
    let mut errors = sheet.errors.clone();
    errors.extend(walk_errors);

    let mut items = inline_items;
    if used_sheet {
        items.extend(sheet_items);
    }

    CompiledStyles {
        items,
        warnings,
        errors,
    }
}

/// `stylesheet! { mod name; … }`: a module holding the sheet and one
/// `view::Class` constant per class, named as the class with `-` as `_`.
pub fn compile_stylesheet(
    css: &str,
    module: &syn::Ident,
    public: bool,
    runtime: &TokenStream,
    class_at: &dyn Fn(&str) -> Span,
) -> CompiledStylesheet {
    let sheet = parse_sheet(css, false, BTreeSet::new());
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
    let tokens = quote! {
        #[allow(non_upper_case_globals)]
        #module_visibility mod #module {
            #(#items)*
            #(#constants)*
        }
    };
    CompiledStylesheet {
        tokens,
        warnings: sheet.warnings,
        errors: sheet.errors,
    }
}

/// `css! { padding: 8px; transition: opacity 120ms }`: one declaration
/// block compiled into `view::InlineStyle` for `El::css`.
pub fn compile_inline(declarations: &str, runtime: &TokenStream) -> CompiledInlineStyle {
    let compiled = compile_inline_css(declarations, &VarScope::new(), &CompileContext::default());
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    push_diags(compiled.diags, &mut warnings, &mut errors);

    let trans_items =
        transition_items_from_compiled(&compiled.transitions, 0..declarations.len(), &mut warnings);

    let patch_stmts = emit_patch_stmts(&compiled.ops, "__nana_patch", false);
    let (patches, site) = if patch_stmts.is_empty() {
        (quote!(), quote!(::core::option::Option::None))
    } else {
        let apply = patch_tokens(&patch_stmts, runtime);
        (
            quote! {
                static __NANA_PATCH_0: #runtime::view::StylePatch =
                    #runtime::view::StylePatch::new(#apply);
            },
            quote! {{
                static __NANA_SITE: #runtime::view::StyleSite =
                    #runtime::view::StyleSite::new(&[(0u64, &__NANA_PATCH_0, ::core::option::Option::None)]);
                ::core::option::Option::Some(&__NANA_SITE)
            }},
        )
    };
    let animate = if trans_items.is_empty() {
        quote!([])
    } else {
        animate_tokens(&trans_items, runtime)
    };
    let tokens = quote! {{
        #patches
        static __NANA_ANIMATE: &[#runtime::view::Implicit] = &#animate;
        #runtime::view::InlineStyle::new(#site, __NANA_ANIMATE)
    }};
    CompiledInlineStyle {
        tokens,
        warnings,
        errors,
    }
}

/// Emit a test verification function that exercises every single `StyleOp`
/// variant emitted by `nana_ui_css::emit_style_op_rust` against
/// `#runtime::view::__patch`, guaranteeing at compile time that all 242
/// emitted setters and type paths resolve and match `StyleOp::apply`.
#[doc(hidden)]
pub fn emit_all_style_ops_verification_fn(runtime: &TokenStream) -> TokenStream {
    let samples = nana_ui_core::sample_style_ops();
    assert_eq!(samples.len(), nana_ui_core::STYLE_OP_COUNT);
    let mut stmts_str = String::new();
    for op in &samples {
        emit_style_op_rust(op, "__nana_patch", &mut stmts_str);
    }
    let stmts: TokenStream = stmts_str
        .parse()
        .expect("all 242 emitted StyleOp statements parse as Rust tokens");
    let count = samples.len();
    quote! {
        fn __apply_all_242_style_ops(__nana_layout: &mut #runtime::LayoutStyle) -> usize {
            #[allow(unused_imports)]
            use #runtime::view::__patch as __nana_patch;
            #stmts
            #count
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(classes: &str, conditional: &[&str]) -> Node {
        let mut attrs = Vec::new();
        if !classes.is_empty() {
            attrs.push(Attr {
                name: AttrName::Plain(format_ident!("class")),
                value: AttrValue::Lit(syn::parse_quote!(#classes)),
            });
        }
        for &c in conditional {
            let ident = format_ident!("{}", c.replace('-', "_"));
            attrs.push(Attr {
                name: AttrName::Directive(format!("class:{c}"), Span::call_site()),
                value: AttrValue::Expr(syn::parse_quote!(#ident)),
            });
        }
        Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs,
            children: Vec::new(),
        })
    }

    #[test]
    fn layer_cascade_order_respects_layers_and_important_reversal() {
        let css = "\
            @layer base, overrides;\n\
            @layer overrides { .box { padding: 20px; opacity: 0.2 !important; } }\n\
            @layer base { .box { padding: 8px; opacity: 0.9 !important; } }\n\
            .box { padding: 12px; }";
        let sheet = parse(css);
        assert!(sheet.errors.is_empty(), "{:?}", sheet.errors);
        assert!(sheet.warnings.is_empty(), "{:?}", sheet.warnings);
        let mut rules: Vec<&Rule> = sheet.rules.iter().collect();
        rules.sort_by_key(|r| r.rank);
        // Expected ascending cascade order (last wins):
        // 1. base normal (padding: 8px)
        // 2. overrides normal (padding: 20px)
        // 3. unlayered normal (padding: 12px)  -> unlayered beats layered!
        // 4. overrides !important (opacity: 0.2)
        // 5. base !important (opacity: 0.9)    -> earlier layer wins on !important!
        assert_eq!(rules.len(), 5);
        assert!(rules[0].patch.contains("8.0"), "{}", rules[0].patch);
        assert!(rules[1].patch.contains("20.0"), "{}", rules[1].patch);
        assert!(rules[2].patch.contains("12.0"), "{}", rules[2].patch);
        assert!(rules[3].patch.contains("0.2"), "{}", rules[3].patch);
        assert!(rules[4].patch.contains("0.9"), "{}", rules[4].patch);
    }

    #[test]
    fn tiered_diagnostics_separate_errors_and_warnings() {
        let css = "\
            .a { padding: 4px; frobnicate: 3; -webkit-font-smoothing: antialiased; }\n\
            .b::before { opacity: 0.5; }\n\
            div > .c { opacity: 1; }";
        let sheet = parse(css);
        assert!(
            sheet
                .errors
                .iter()
                .any(|e| e.message.contains("frobnicate")),
            "unknown property must be an error: {:?}",
            sheet.errors
        );
        assert!(
            sheet
                .warnings
                .iter()
                .any(|w| w.message.contains("-webkit-font-smoothing")),
            "vendor-prefixed property must be a warning: {:?}",
            sheet.warnings
        );
        assert!(
            sheet
                .warnings
                .iter()
                .any(|w| w.message.contains("pseudo-element")),
            "{:?}",
            sheet.warnings
        );
        assert!(
            sheet
                .warnings
                .iter()
                .any(|w| w.message.contains("class selectors")),
            "{:?}",
            sheet.warnings
        );
    }

    #[test]
    fn dynamic_theme_tokens_and_dual_root_vars_emit_light_dark() {
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
        assert!(sheet.errors.is_empty(), "{:?}", sheet.errors);
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
        assert!(
            tokens.contains("ThemePair") || tokens.contains("PaintColor::light_dark"),
            "{tokens}"
        );
    }

    #[test]
    fn ancestor_interactive_emits_track_and_ancestor_pseudo() {
        let css = ".card:hover .icon { opacity: 0.5; }";
        let mut nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![Attr {
                name: AttrName::Plain(format_ident!("class")),
                value: AttrValue::Lit(syn::parse_quote!("card")),
            }],
            children: vec![element("icon", &[])],
        })];
        let compiled = compile_styles(css, &mut nodes, &quote!(x));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        let expanded = crate::expand(&quote!(x), &nodes).unwrap().to_string();
        assert!(expanded.contains("interactive_state"), "{expanded}");
        assert!(expanded.contains("track_interactive_state"), "{expanded}");
        assert!(expanded.contains("ancestor_pseudo"), "{expanded}");
    }

    #[test]
    fn element_scoped_custom_properties_inherit_down_tree() {
        let css = "\
            .dark-card { --bg: #111111; }\n\
            .dark-card .title { color: var(--bg); }";
        let mut nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![Attr {
                name: AttrName::Plain(format_ident!("class")),
                value: AttrValue::Lit(syn::parse_quote!("dark-card")),
            }],
            children: vec![element("title", &[])],
        })];
        let compiled = compile_styles(css, &mut nodes, &quote!(x));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        let items: String = compiled.items.iter().map(ToString::to_string).collect();
        assert!(items.contains("paint_colors :: color"), "{items}");
    }

    #[test]
    fn media_and_font_face_emit_on_sheet() {
        let css = "\
            @font-face { font-family: \"TestFont\"; src: local(\"TestFont\"); }\n\
            @media (min-width: 600px) { .card { padding: 24px; } }";
        let mut nodes = vec![element("card", &[])];
        let compiled = compile_styles(css, &mut nodes, &quote!(x));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        let items: String = compiled.items.iter().map(ToString::to_string).collect();
        assert!(items.contains("with_media"), "{items}");
        assert!(items.contains("SheetMediaQuery :: new"), "{items}");
        assert!(items.contains("with_font_faces"), "{items}");
        assert!(items.contains("FontFaceRule"), "{items}");
        let expanded = crate::expand(&quote!(x), &nodes).unwrap().to_string();
        assert!(expanded.contains(". sheet (& __NANA_SHEET)"), "{expanded}");
    }

    #[test]
    fn inline_style_attr_emits_inline_patch_and_preserves_span_style() {
        let mut nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![Attr {
                name: AttrName::Plain(format_ident!("style")),
                value: AttrValue::Lit(syn::parse_quote!("padding: 10px; opacity: 0.5;")),
            }],
            children: Vec::new(),
        })];
        let compiled = compile_styles("", &mut nodes, &quote!(x));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        let expanded = crate::expand(&quote!(x), &nodes).unwrap().to_string();
        assert!(expanded.contains("inline_patch"), "{expanded}");
    }

    #[test]
    fn class_when_avoids_double_closure_and_allows_bool_literal() {
        let css = ".card.active { opacity: 0.5; }";
        let mut nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![
                Attr {
                    name: AttrName::Plain(format_ident!("class")),
                    value: AttrValue::Lit(syn::parse_quote!("card")),
                },
                Attr {
                    name: AttrName::Directive("class:active".into(), Span::call_site()),
                    value: AttrValue::Expr(syn::parse_quote!(move || flag.get())),
                },
            ],
            children: Vec::new(),
        })];
        let compiled = compile_styles(css, &mut nodes, &quote!(x));
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let expanded = crate::expand(&quote!(x), &nodes).unwrap().to_string();
        assert!(!expanded.contains("move || move ||"), "{expanded}");

        let mut lit_nodes = vec![Node::Element(Element {
            module: Vec::new(),
            name: format_ident!("Column"),
            attrs: vec![
                Attr {
                    name: AttrName::Plain(format_ident!("class")),
                    value: AttrValue::Lit(syn::parse_quote!("card")),
                },
                Attr {
                    name: AttrName::Directive("class:active".into(), Span::call_site()),
                    value: AttrValue::Lit(syn::parse_quote!(true)),
                },
            ],
            children: Vec::new(),
        })];
        let compiled_lit = compile_styles(css, &mut lit_nodes, &quote!(x));
        assert!(compiled_lit.errors.is_empty(), "{:?}", compiled_lit.errors);
        let expanded_lit = crate::expand(&quote!(x), &lit_nodes).unwrap().to_string();
        assert!(expanded_lit.contains("class_when"), "{expanded_lit}");
        assert!(expanded_lit.contains(", true)"), "{expanded_lit}");
    }

    #[test]
    fn compile_styles_hot_emits_hot_style_patch_from_ops_and_keeps_shape_stable() {
        let mut nodes_a = vec![element("card", &["open"])];
        let a = compile_styles_hot(
            ".card { padding: 4px; opacity: 0.9; }\n.card.open { opacity: 0.5; }",
            &mut nodes_a,
            &quote!(x),
            "Card",
        );
        assert!(a.errors.is_empty(), "{:?}", a.errors);
        assert!(!a.hot_sheet.rules[0].patch.ops.is_empty());

        let mut nodes_b = vec![element("card", &["open"])];
        let b = compile_styles_hot(
            ".card { padding: 16px; }\n.card.open { opacity: 1; }",
            &mut nodes_b,
            &quote!(x),
            "Card",
        );
        assert_eq!(a.style_shape.to_string(), b.style_shape.to_string());
        assert_ne!(a.hot_sheet, b.hot_sheet);
        assert_eq!(b.hot_sheet.rules.len(), 2);

        let mut nodes_c = vec![element("card", &["open"])];
        let c = compile_styles_hot(
            ".card { padding: 16px; transition: opacity 150ms; }",
            &mut nodes_c,
            &quote!(x),
            "Card",
        );
        assert_ne!(b.style_shape.to_string(), c.style_shape.to_string());
    }
}
