//! Direct CSS declaration -> `Vec<nana_ui_core::StyleOp>` compilation.
//!
//! Zero `witness_layout`, zero `CssPaintColor`, zero thread-locals.
//! Every property parser accepts `&[ComponentValue]` (after `syntax::vars::expand_vars`)
//! and returns `Result<Vec<StyleOp>, CssDiag>` so invalid declarations NEVER clobber
//! prior cascaded values and empty values NEVER panic.

pub mod grid;
pub mod layout;
pub mod motion;
pub mod paint;
pub mod pseudo;
pub mod text;
pub mod theme;
pub mod transform;

use nana_ui_core::{
    BackgroundImage, BackgroundImageFit, BackgroundPosition, BackgroundRepeat, BorderImageSpec,
    DisplaySpec, FlexDirection, FontSizeContext, LayoutStyle, LengthSpec, LogicalBoxEdges,
    ScrollbarSkin, StyleOp, TextStrokeSpec,
};

use crate::cascade::CustomPropDecl;
use crate::properties::grid::{is_grid_track_property, parse_grid_property};
use crate::properties::layout::parse_layout_property;
use crate::properties::motion::{
    CompiledAnimation, CompiledTransition, MotionBuilder, is_motion_property,
};
use crate::properties::paint::parse_paint_property;
use crate::properties::pseudo::compile_pseudo_property;
use crate::properties::text::parse_text_property;
use crate::properties::theme::try_parse_theme_property;
use crate::properties::transform::parse_transform_property;
use crate::syntax::diag::{CssDiag, diagnose_unknown_property};
use crate::syntax::parser::{ComponentValue, Declaration, parse_declaration_list, trim_ws};
use crate::syntax::selector::PseudoElement;
use crate::syntax::span::Span;
use crate::syntax::tokenizer::{SpannedToken, Token};
use crate::syntax::values::ColorParseOptions;
use crate::syntax::vars::{VarScope, expand_vars};

/// Explicit, thread-local-free compilation context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompileContext {
    pub color_opts: ColorParseOptions,
    pub font_sizes: FontSizeContext,
    pub viewport: Option<(f32, f32)>,
    /// Keep unresolved `--nana-*` theme tokens as `var(--nana-*)` so color
    /// and theme-expression parsers lower them to runtime-dynamic theme roles.
    pub preserve_nana_tokens: bool,
}

impl Default for CompileContext {
    fn default() -> Self {
        Self {
            color_opts: ColorParseOptions {
                dark_scheme: false,
                dynamic_light_dark: true,
            },
            font_sizes: FontSizeContext::default(),
            viewport: None,
            preserve_nana_tokens: true,
        }
    }
}

impl CompileContext {
    /// Context for static evaluation (e.g. `tools/css-parity` or tests) with
    /// explicit viewport and light/dark scheme.
    pub fn static_eval(viewport: Option<(f32, f32)>, dark_scheme: bool) -> Self {
        Self {
            color_opts: ColorParseOptions {
                dark_scheme,
                dynamic_light_dark: false,
            },
            font_sizes: FontSizeContext::default(),
            viewport,
            preserve_nana_tokens: true,
        }
    }

    /// Resolve a [`LengthSpec`] to px using this context's viewport and font sizes.
    pub fn resolve_length_px(&self, spec: LengthSpec) -> Option<f32> {
        spec.resolve_with_fonts(None, self.viewport, self.font_sizes)
    }
}

/// Output of compiling a slice of CSS [`Declaration`]s.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompiledDecls {
    pub ops: Vec<StyleOp>,
    pub transitions: Vec<CompiledTransition>,
    pub animations: Vec<CompiledAnimation>,
    pub custom_props: Vec<CustomPropDecl>,
}

/// Output of compiling inline CSS text (`.css(css!{...})` or `style="..."`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompiledInline {
    pub ops: Vec<StyleOp>,
    pub transitions: Vec<CompiledTransition>,
    pub animations: Vec<CompiledAnimation>,
    pub custom_props: Vec<CustomPropDecl>,
    pub diags: Vec<CssDiag>,
}

/// Internal per-declaration-block coordination state (purely local, no LayoutStyle
/// mutation on error, no thread-locals).
#[derive(Default)]
pub(crate) struct DeclState {
    pub display: Option<DisplaySpec>,
    pub direction: Option<FlexDirection>,
    pub saw_align_items: bool,
    pub grid_has_columns: bool,
    pub grid_has_rows: bool,
    pub logical_edges: LogicalBoxEdges,
    pub dirty_padding_edges: bool,
    pub dirty_margin_edges: bool,
    pub dirty_inset_edges: bool,
    pub bg_layers: Vec<BackgroundImage>,
    pub bg_size_list: Vec<BackgroundImageFit>,
    pub bg_size_lengths: Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
    pub bg_position_list: Vec<BackgroundPosition>,
    pub bg_repeat_list: Vec<BackgroundRepeat>,
    pub border_radii: Option<[LengthSpec; 4]>,
    pub border_image: Option<BorderImageSpec>,
    pub unsupported_border_image: bool,
    pub text_stroke: Option<TextStrokeSpec>,
    pub scrollbar: Option<ScrollbarSkin>,
}

/// Compile a slice of parsed [`Declaration`]s into typed [`StyleOp`]s, motion
/// specs, and element-scoped `--*` custom properties.
///
/// - Expands `var(...)` via `syntax::vars::expand_vars` on ALL properties
///   (including arbitrarily nested fallbacks `var(--a, var(--b, 100px))`).
/// - Applies `direction`, `writing-mode`, and `text-orientation` first so
///   logical box properties in the same block resolve against the used writing
///   context, while preserving exact declaration source order for everything else.
/// - Invalid declarations emit structured [`CssDiag`]s and NEVER emit partial
///   ops or clobber prior values.
pub fn compile_declarations(
    decls: &[Declaration],
    vars: &VarScope,
    pseudo: Option<&PseudoElement>,
    ctx: &CompileContext,
    diags: &mut Vec<CssDiag>,
) -> CompiledDecls {
    let mut ops = Vec::new();
    let mut custom_props = Vec::new();
    let mut motion = MotionBuilder::default();
    let mut state = DeclState::default();

    // First extract custom properties & separate writing-mode/direction hints.
    let mut ordered_indices: Vec<usize> = Vec::with_capacity(decls.len());
    let mut other_indices: Vec<usize> = Vec::with_capacity(decls.len());
    for (idx, decl) in decls.iter().enumerate() {
        if decl.name.starts_with("--") {
            custom_props.push(CustomPropDecl {
                name: decl.name.clone(),
                value: decl.value.clone(),
                important: decl.important,
                span: decl.span,
            });
            continue;
        }
        let norm = normalize_prop_name(&decl.name);
        if matches!(
            norm.as_str(),
            "direction" | "writing-mode" | "text-orientation"
        ) {
            ordered_indices.push(idx);
        } else {
            other_indices.push(idx);
        }
    }
    ordered_indices.extend(other_indices);

    for idx in ordered_indices {
        let decl = &decls[idx];
        let prop = normalize_prop_name(&decl.name);

        // Expand `var(...)` directly on ComponentValue tree.
        let expanded = match expand_vars(&decl.value, vars, ctx.preserve_nana_tokens, diags) {
            Ok(v) => v,
            Err(_) => {
                // For grid track lists, unresolved `var(--x)` degrades to `auto`
                // so track counts remain intact.
                if is_grid_track_property(&prop) {
                    expand_grid_vars_fallback(&decl.value, vars)
                } else {
                    continue;
                }
            }
        };

        let trimmed = trim_ws(&expanded);
        if trimmed.is_empty() {
            diags.push(CssDiag::error(
                decl.value_span,
                crate::syntax::diag::DiagCode::InvalidValue,
                format!("empty value for CSS property `{prop}`"),
            ));
            continue;
        }

        // Pseudo-element dispatch (`::placeholder`, `::selection`, `::-webkit-scrollbar*`).
        if let Some(pe) = pseudo {
            match compile_pseudo_property(pe, &prop, trimmed, decl.span, ctx, &mut state.scrollbar)
            {
                Ok(mut emitted) => ops.append(&mut emitted),
                Err(d) => diags.push(d),
            }
            continue;
        }

        // Motion properties (`transition*`, `animation*`) in exact declaration order.
        if is_motion_property(&prop) {
            if let Err(d) = motion.apply(&prop, trimmed, decl.span) {
                diags.push(d);
            }
            continue;
        }

        // Theme token expressions (`theme(...)` / `var(--nana-*)` on layout/spacing/type props).
        if let Some(res) = try_parse_theme_property(&prop, trimmed, decl.span) {
            match res {
                Ok(mut emitted) => ops.append(&mut emitted),
                Err(d) => diags.push(d),
            }
            continue;
        }

        // Standard property dispatch.
        if let Some(res) = parse_single_property(&prop, trimmed, decl.span, ctx, &mut state) {
            match res {
                Ok(mut emitted) => ops.append(&mut emitted),
                Err(d) => diags.push(d),
            }
            continue;
        }

        // Unknown or vendor-prefixed property -> tiered diagnostic (Warning vs Error).
        diags.push(diagnose_unknown_property(&decl.name, decl.name_span));
    }

    if let Some(skin) = state.scrollbar {
        ops.push(StyleOp::Scrollbar(Some(skin)));
    }

    let (transitions, animations) = motion.finish();
    CompiledDecls {
        ops,
        transitions,
        animations,
        custom_props,
    }
}

/// Compile inline CSS declarations (`width: 100px; color: red;`) into
/// [`CompiledInline`].
pub fn compile_inline_css(css: &str, vars: &VarScope, ctx: &CompileContext) -> CompiledInline {
    let mut diags = Vec::new();
    let decls = parse_declaration_list(css, &mut diags);
    // Inline custom properties defined earlier in the same style attribute are
    // available to later declarations in the same attribute.
    let mut local_scope = if vars.is_empty() {
        VarScope::new()
    } else {
        VarScope::with_parent(std::sync::Arc::new(vars.clone()))
    };
    for decl in &decls {
        if decl.name.starts_with("--") {
            local_scope.set(decl.name.clone(), decl.value.clone());
        }
    }
    let compiled = compile_declarations(&decls, &local_scope, None, ctx, &mut diags);
    CompiledInline {
        ops: compiled.ops,
        transitions: compiled.transitions,
        animations: compiled.animations,
        custom_props: compiled.custom_props,
        diags,
    }
}

/// Apply a slice of [`StyleOp`]s onto a [`LayoutStyle`] and resolve any logical
/// box edges against the final writing context.
pub fn apply_style_ops(layout: &mut LayoutStyle, ops: &[StyleOp]) {
    for op in ops {
        op.apply(layout);
    }
    layout.resolve_logical_box_edges();
}

/// Parse and apply inline CSS text onto `layout` using default static evaluation.
pub fn apply_css_text(layout: &mut LayoutStyle, css: &str) {
    let vars = VarScope::new();
    let ctx = CompileContext::static_eval(None, false);
    let compiled = compile_inline_css(css, &vars, &ctx);
    apply_style_ops(layout, &compiled.ops);
}

/// Parse and apply inline CSS text onto `layout` with an explicit [`CompileContext`]
/// and [`VarScope`].
pub fn apply_css_text_with(
    layout: &mut LayoutStyle,
    css: &str,
    vars: &VarScope,
    ctx: &CompileContext,
) -> Vec<CssDiag> {
    let compiled = compile_inline_css(css, vars, ctx);
    apply_style_ops(layout, &compiled.ops);
    compiled.diags
}

/// Evaluate whether `(property, value)` is supported by this CSS engine
/// (used by `@supports (prop: value)`).
pub fn is_supported_declaration(
    property: &str,
    value: &[ComponentValue],
    ctx: &CompileContext,
) -> bool {
    let prop = normalize_prop_name(property);
    if prop.starts_with("--") {
        return !trim_ws(value).is_empty();
    }
    let trimmed = trim_ws(value);
    if trimmed.is_empty() {
        return false;
    }
    if is_motion_property(&prop) {
        let mut mb = MotionBuilder::default();
        return mb.apply(&prop, trimmed, Span::EMPTY).is_ok();
    }
    if let Some(res) = try_parse_theme_property(&prop, trimmed, Span::EMPTY) {
        return res.is_ok();
    }
    let mut state = DeclState::default();
    matches!(
        parse_single_property(&prop, trimmed, Span::EMPTY, ctx, &mut state),
        Some(Ok(_))
    )
}

fn parse_single_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    if let Some(res) = parse_layout_property(prop, values, span, ctx, state) {
        return Some(res);
    }
    if let Some(res) = parse_grid_property(prop, values, span, ctx, state) {
        return Some(res);
    }
    if let Some(res) = parse_text_property(prop, values, span, ctx, state) {
        return Some(res);
    }
    if let Some(res) = parse_paint_property(prop, values, span, ctx, state) {
        return Some(res);
    }
    if let Some(res) = parse_transform_property(prop, values, span, ctx) {
        return Some(res);
    }
    None
}

fn normalize_prop_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("--") {
        return trimmed.to_string();
    }
    if trimmed.contains('_') || trimmed.bytes().any(|b| b.is_ascii_uppercase()) {
        let mut out = String::with_capacity(trimmed.len() + 4);
        for (i, ch) in trimmed.chars().enumerate() {
            if ch == '_' {
                out.push('-');
            } else if ch.is_ascii_uppercase() {
                if i > 0 && !out.ends_with('-') {
                    out.push('-');
                }
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

/// For grid track lists where one `var(--missing)` fails expansion, replace
/// only unresolved `var(...)` nodes with `auto` while expanding resolvable ones.
fn expand_grid_vars_fallback(values: &[ComponentValue], vars: &VarScope) -> Vec<ComponentValue> {
    let mut out = Vec::with_capacity(values.len());
    for cv in values {
        match cv {
            ComponentValue::Function { name, span, .. } if name.eq_ignore_ascii_case("var") => {
                let mut local_diags = Vec::new();
                match expand_vars(std::slice::from_ref(cv), vars, false, &mut local_diags) {
                    Ok(expanded) => out.extend(expanded),
                    Err(_) => out.push(ComponentValue::Token(SpannedToken::new(
                        Token::Ident("auto".into()),
                        *span,
                    ))),
                }
            }
            ComponentValue::Function { name, span, args } => {
                out.push(ComponentValue::Function {
                    name: name.clone(),
                    span: *span,
                    args: expand_grid_vars_fallback(args, vars),
                });
            }
            ComponentValue::SimpleBlock { kind, span, values } => {
                out.push(ComponentValue::SimpleBlock {
                    kind: *kind,
                    span: *span,
                    values: expand_grid_vars_fallback(values, vars),
                });
            }
            other => out.push(other.clone()),
        }
    }
    out
}
