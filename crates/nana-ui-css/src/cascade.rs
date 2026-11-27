//! Stylesheet parsing, CSS Nesting Level 1 desugaring, CSS Cascade 5 `@layer`
//! ordering, `@media` / `@container` / `@supports` / `@keyframes` / `@font-face`
//! handling, and custom property extraction (`--*`).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use nana_ui_core::{FontFaceRule, MediaEnv, MediaQueryList};

use crate::lower::{
    ContainerQuery, ContainerQueryUnsupported, lower_container_query, lower_font_face_rule,
    lower_media_query_list,
};
use crate::properties::{CompileContext, is_supported_declaration};
use crate::syntax::MAX_NESTING;
use crate::syntax::at_rule::{
    KeyframesRule, LayerName, LayerOrder, LayerPrelude, LayerRegistry, compare_layer_cascade,
    parse_container_prelude, parse_font_face_rule, parse_keyframes_rule, parse_layer_prelude,
    parse_media_query_list, parse_supports_condition,
};
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{
    AtRule, ComponentValue, Declaration, QualifiedRule, Rule, StyleBlockItem, parse_rule_list,
    parse_style_block, parse_stylesheet as parse_syntax_stylesheet, trim_ws,
};
use crate::syntax::selector::{
    AttrOperator, Combinator, ComplexSelector, CompoundSelector, PseudoClass, SelectorList,
    Specificity, TypeSelector, parse_selector_list,
};
use crate::syntax::span::Span;
use crate::syntax::vars::{VarCascadeKey, VarScope};

/// Full CSS Cascade 5 priority key:
/// 1. Normal layered (ascending `layer`) < Normal unlayered < `!important` unlayered
///    < `!important` layered (reversed `layer` — earlier declared layer wins!)
/// 2. Higher [`Specificity`] wins
/// 3. Later `source_order` wins
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CascadeKey {
    pub layer: LayerOrder,
    pub important: bool,
    pub specificity: Specificity,
    pub source_order: u32,
}

impl CascadeKey {
    pub const fn new(
        layer: LayerOrder,
        important: bool,
        specificity: Specificity,
        source_order: u32,
    ) -> Self {
        Self {
            layer,
            important,
            specificity,
            source_order,
        }
    }

    /// Convert to [`VarCascadeKey`] for inserting custom properties into a [`VarScope`].
    pub const fn to_var_key(self) -> VarCascadeKey {
        VarCascadeKey {
            important: self.important,
            layer: self.layer,
            specificity: self.specificity,
            source_order: self.source_order,
        }
    }
}

impl Ord for CascadeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let layer_cmp =
            compare_layer_cascade(self.important, self.layer, other.important, other.layer);
        if layer_cmp != Ordering::Equal {
            return layer_cmp;
        }
        let spec_cmp = self.specificity.cmp(&other.specificity);
        if spec_cmp != Ordering::Equal {
            return spec_cmp;
        }
        self.source_order.cmp(&other.source_order)
    }
}

impl PartialOrd for CascadeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One element-scoped custom property declaration (`--name: value [!important]`)
/// preserved verbatim as [`ComponentValue`] tokens for ancestor-chain [`VarScope`]
/// resolution in build-time view codegen.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPropDecl {
    pub name: String,
    pub value: Vec<ComponentValue>,
    pub important: bool,
    pub span: Span,
}

/// One style rule in a [`ParsedSheet`] with its selector list, cascade metadata,
/// ordinary declarations, custom properties, and enclosing `@media` / `@container`
/// guards.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRule {
    pub selectors: SelectorList,
    pub layer: LayerOrder,
    pub source_order: u32,
    pub declarations: Vec<Declaration>,
    pub custom_props: Vec<CustomPropDecl>,
    /// Enclosing `@media` query lists (every entry must match for this rule to apply).
    pub media: Vec<MediaQueryList>,
    /// Enclosing `@container` query guard, if inside `@container`.
    pub container: Option<Result<ContainerQuery, ContainerQueryUnsupported>>,
    pub span: Span,
}

impl ParsedRule {
    /// Compute the [`CascadeKey`] for a specific `selector` in [`Self::selectors`]
    /// and a declaration's `important` flag.
    pub fn cascade_key(&self, selector: &ComplexSelector, important: bool) -> CascadeKey {
        CascadeKey {
            layer: self.layer,
            important,
            specificity: selector.specificity(),
            source_order: self.source_order,
        }
    }

    /// Whether all enclosing `@media` guards match `env`.
    pub fn matches_media(&self, env: &MediaEnv) -> bool {
        self.media.iter().all(|list| list.matches(env))
    }
}

/// Complete parsed stylesheet ready for build-time codegen or runtime evaluation.
#[derive(Debug, Clone, Default)]
pub struct ParsedSheet {
    pub rules: Vec<ParsedRule>,
    pub keyframes: BTreeMap<String, KeyframesRule>,
    pub font_faces: Vec<FontFaceRule>,
    pub layers: LayerRegistry,
    pub diags: Vec<CssDiag>,
}

impl ParsedSheet {
    /// Return all rules whose `@media` guards match `env`.
    pub fn active_rules(&self, env: &MediaEnv) -> Vec<&ParsedRule> {
        self.rules.iter().filter(|r| r.matches_media(env)).collect()
    }

    /// Build the document-level [`VarScope`] from unconditional rules targeting
    /// `:root`, `html`, `body`, `*`, or `[data-theme="..."]` matching `theme`
    /// (`"light"` or `"dark"`), respecting full CSS Cascade 5 layer/specificity/
    /// `!important` ordering.
    pub fn root_var_scope(&self, theme: &str) -> VarScope {
        let mut scope = VarScope::new();
        for rule in &self.rules {
            if !rule.media.is_empty() || rule.container.is_some() || rule.custom_props.is_empty() {
                continue;
            }
            let best_spec = rule
                .selectors
                .0
                .iter()
                .filter(|sel| selector_matches_document_root(sel, theme))
                .map(ComplexSelector::specificity)
                .max();
            let Some(spec) = best_spec else {
                continue;
            };
            for cp in &rule.custom_props {
                let key = CascadeKey::new(rule.layer, cp.important, spec, rule.source_order);
                scope.set_with_cascade(cp.name.clone(), cp.value.clone(), key.to_var_key());
            }
        }
        scope
    }
}

/// Parse a full CSS stylesheet into a [`ParsedSheet`].
pub fn parse_stylesheet(css: &str) -> ParsedSheet {
    parse_stylesheet_with_base(css, None)
}

/// Parse a full CSS stylesheet with an originating `base_href` for `@font-face`
/// relative URL resolution.
pub fn parse_stylesheet_with_base(css: &str, base_href: Option<&str>) -> ParsedSheet {
    let raw = parse_syntax_stylesheet(css);
    let mut sheet = ParsedSheet {
        diags: raw.diags,
        ..ParsedSheet::default()
    };
    let mut state = LowerState {
        sheet: &mut sheet,
        base_href,
        next_order: 0,
    };
    let ctx = RuleContext::default();
    state.visit_rule_list(&raw.rules, &ctx, 0);
    sheet
}

#[derive(Clone)]
struct RuleContext {
    layer_prefix: Option<LayerName>,
    layer_order: LayerOrder,
    media: Vec<MediaQueryList>,
    container: Option<Result<ContainerQuery, ContainerQueryUnsupported>>,
    parent_selectors: Option<SelectorList>,
}

impl Default for RuleContext {
    fn default() -> Self {
        Self {
            layer_prefix: None,
            layer_order: LayerOrder::UNLAYERED,
            media: Vec::new(),
            container: None,
            parent_selectors: None,
        }
    }
}

struct LowerState<'a> {
    sheet: &'a mut ParsedSheet,
    base_href: Option<&'a str>,
    next_order: u32,
}

impl LowerState<'_> {
    fn alloc_order(&mut self) -> u32 {
        let id = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        id
    }

    fn visit_rule_list(&mut self, rules: &[Rule], ctx: &RuleContext, depth: usize) {
        if depth >= MAX_NESTING {
            if let Some(first) = rules.first() {
                self.sheet.diags.push(CssDiag::error(
                    first.span(),
                    DiagCode::NestingLimitExceeded,
                    format!("stylesheet rule nesting limit ({MAX_NESTING}) exceeded"),
                ));
            }
            return;
        }
        for rule in rules {
            match rule {
                Rule::Qualified(q) => self.visit_qualified_rule(q, ctx, depth),
                Rule::At(at) => self.visit_at_rule(at, ctx, depth),
            }
        }
    }

    fn visit_qualified_rule(&mut self, q: &QualifiedRule, ctx: &RuleContext, depth: usize) {
        let Some(parsed_sels) = parse_selector_list(&q.prelude, &mut self.sheet.diags) else {
            return;
        };
        let selectors = match &ctx.parent_selectors {
            Some(parents) => desugar_nested_selectors(parents, &parsed_sels),
            None => parsed_sels,
        };
        if selectors.0.is_empty() {
            return;
        }

        let items = parse_style_block(&q.block, &mut self.sheet.diags);
        let mut declarations = Vec::new();
        let mut custom_props = Vec::new();
        let mut nested_rules = Vec::new();

        for item in items {
            match item {
                StyleBlockItem::Declaration(decl) => {
                    if decl.name.starts_with("--") {
                        custom_props.push(CustomPropDecl {
                            name: decl.name,
                            value: decl.value,
                            important: decl.important,
                            span: decl.span,
                        });
                    } else {
                        declarations.push(decl);
                    }
                }
                StyleBlockItem::NestedRule(nested) => nested_rules.push(nested),
            }
        }

        let order = self.alloc_order();
        if !declarations.is_empty() || !custom_props.is_empty() {
            self.sheet.rules.push(ParsedRule {
                selectors: selectors.clone(),
                layer: ctx.layer_order,
                source_order: order,
                declarations,
                custom_props,
                media: ctx.media.clone(),
                container: ctx.container.clone(),
                span: q.span,
            });
        }

        if !nested_rules.is_empty() {
            let mut child_ctx = ctx.clone();
            child_ctx.parent_selectors = Some(selectors);
            self.visit_rule_list(&nested_rules, &child_ctx, depth + 1);
        }
    }

    fn visit_at_rule(&mut self, at: &AtRule, ctx: &RuleContext, depth: usize) {
        let name = at.name.to_ascii_lowercase();
        match name.as_str() {
            "charset" => {}
            "layer" => self.visit_layer(at, ctx, depth),
            "media" => self.visit_media(at, ctx, depth),
            "container" => self.visit_container(at, ctx, depth),
            "supports" => self.visit_supports(at, ctx, depth),
            "keyframes" | "-webkit-keyframes" => {
                if let Some(kf) = parse_keyframes_rule(at, &mut self.sheet.diags) {
                    self.sheet.keyframes.insert(kf.name.clone(), kf);
                }
            }
            "font-face" => {
                if let Some(syntax_ff) = parse_font_face_rule(at, &mut self.sheet.diags)
                    && let Some(core_ff) = lower_font_face_rule(&syntax_ff, self.base_href)
                {
                    self.sheet.font_faces.push(core_ff);
                }
            }
            _ => {
                self.sheet.diags.push(CssDiag::error(
                    at.span,
                    DiagCode::UnsupportedAtRule,
                    format!("unsupported at-rule `@{}`", at.name),
                ));
            }
        }
    }

    fn visit_layer(&mut self, at: &AtRule, ctx: &RuleContext, depth: usize) {
        let Some(prelude) = parse_layer_prelude(at, &mut self.sheet.diags) else {
            return;
        };
        match prelude {
            LayerPrelude::Statement(names) => {
                for name in names {
                    let full = match &ctx.layer_prefix {
                        Some(prefix) => prefix.join(&name),
                        None => name,
                    };
                    self.sheet.layers.declare(&full);
                }
            }
            LayerPrelude::Block(opt_name) => {
                let Some(block) = &at.block else { return };
                let (new_prefix, new_order) = match opt_name {
                    Some(name) => {
                        let full = match &ctx.layer_prefix {
                            Some(prefix) => prefix.join(&name),
                            None => name,
                        };
                        let order = self.sheet.layers.declare(&full);
                        (Some(full), order)
                    }
                    None => {
                        let order = self.sheet.layers.declare_anonymous();
                        (ctx.layer_prefix.clone(), order)
                    }
                };
                let mut child_ctx = ctx.clone();
                child_ctx.layer_prefix = new_prefix;
                child_ctx.layer_order = new_order;
                self.visit_block_with_nesting(block, &child_ctx, depth + 1, at.span);
            }
        }
    }

    fn visit_media(&mut self, at: &AtRule, ctx: &RuleContext, depth: usize) {
        let Some(block) = &at.block else {
            self.sheet.diags.push(CssDiag::error(
                at.span,
                DiagCode::InvalidAtRule,
                "@media requires a `{ ... }` block",
            ));
            return;
        };
        let syntax_list = parse_media_query_list(&at.prelude, &mut self.sheet.diags);
        let core_list = lower_media_query_list(&syntax_list, &mut self.sheet.diags);
        let mut child_ctx = ctx.clone();
        child_ctx.media.push(core_list);
        self.visit_block_with_nesting(block, &child_ctx, depth + 1, at.span);
    }

    fn visit_container(&mut self, at: &AtRule, ctx: &RuleContext, depth: usize) {
        let Some(block) = &at.block else {
            self.sheet.diags.push(CssDiag::error(
                at.span,
                DiagCode::InvalidAtRule,
                "@container requires a `{ ... }` block",
            ));
            return;
        };
        let guard = if ctx.container.is_some() {
            self.sheet.diags.push(CssDiag::error(
                at.span,
                DiagCode::InvalidAtRule,
                "nested `@container` queries are unsupported",
            ));
            Err(ContainerQueryUnsupported::Nested)
        } else {
            // Check top-level comma (condition list is unsupported).
            let has_comma = at
                .prelude
                .iter()
                .any(|cv| matches!(cv.as_token(), Some(crate::syntax::Token::Comma)));
            if has_comma {
                Err(ContainerQueryUnsupported::ConditionList)
            } else {
                match parse_container_prelude(&at.prelude, &mut self.sheet.diags) {
                    Some((name, cond)) => lower_container_query(name, &cond),
                    None => {
                        let trimmed = trim_ws(&at.prelude);
                        if trimmed.len() == 1 && trimmed[0].as_ident().is_some() {
                            Err(ContainerQueryUnsupported::NoCondition)
                        } else {
                            Err(ContainerQueryUnsupported::Invalid)
                        }
                    }
                }
            }
        };
        let mut child_ctx = ctx.clone();
        child_ctx.container = Some(guard);
        self.visit_block_with_nesting(block, &child_ctx, depth + 1, at.span);
    }

    fn visit_supports(&mut self, at: &AtRule, ctx: &RuleContext, depth: usize) {
        let Some(block) = &at.block else {
            self.sheet.diags.push(CssDiag::error(
                at.span,
                DiagCode::InvalidAtRule,
                "@supports requires a `{ ... }` block",
            ));
            return;
        };
        let Some(cond) = parse_supports_condition(&at.prelude, &mut self.sheet.diags) else {
            return;
        };
        let compile_ctx = CompileContext::default();
        let holds = cond.evaluate(
            &|prop, val| is_supported_declaration(prop, val, &compile_ctx),
            &|sel_cvs| {
                let mut sink = Vec::new();
                parse_selector_list(sel_cvs, &mut sink).is_some() && sink.is_empty()
            },
        );
        if holds {
            self.visit_block_with_nesting(block, ctx, depth + 1, at.span);
        }
    }

    /// Inside a conditional at-rule (`@media`, `@container`, `@supports`, `@layer`),
    /// if we are nested inside a style rule (`ctx.parent_selectors.is_some()`),
    /// bare declarations belong to the parent selector per CSS Nesting Level 1.
    fn visit_block_with_nesting(
        &mut self,
        block: &[ComponentValue],
        ctx: &RuleContext,
        depth: usize,
        span: Span,
    ) {
        if let Some(parent_sels) = &ctx.parent_selectors {
            let items = parse_style_block(block, &mut self.sheet.diags);
            let mut decls = Vec::new();
            let mut custom_props = Vec::new();
            let mut nested_rules = Vec::new();
            for item in items {
                match item {
                    StyleBlockItem::Declaration(d) => {
                        if d.name.starts_with("--") {
                            custom_props.push(CustomPropDecl {
                                name: d.name,
                                value: d.value,
                                important: d.important,
                                span: d.span,
                            });
                        } else {
                            decls.push(d);
                        }
                    }
                    StyleBlockItem::NestedRule(r) => nested_rules.push(r),
                }
            }
            if !decls.is_empty() || !custom_props.is_empty() {
                let order = self.alloc_order();
                self.sheet.rules.push(ParsedRule {
                    selectors: parent_sels.clone(),
                    layer: ctx.layer_order,
                    source_order: order,
                    declarations: decls,
                    custom_props,
                    media: ctx.media.clone(),
                    container: ctx.container.clone(),
                    span,
                });
            }
            if !nested_rules.is_empty() {
                self.visit_rule_list(&nested_rules, ctx, depth);
            }
        } else {
            let inner_rules = parse_rule_list(block, &mut self.sheet.diags);
            self.visit_rule_list(&inner_rules, ctx, depth);
        }
    }
}

// ============================================================================
// CSS Nesting Level 1 selector desugaring
// ============================================================================

fn desugar_nested_selectors(parents: &SelectorList, children: &SelectorList) -> SelectorList {
    let mut out = Vec::new();
    for parent in &parents.0 {
        for child in &children.0 {
            out.push(combine_nested_selector(parent, child));
        }
    }
    SelectorList(out)
}

fn combine_nested_selector(parent: &ComplexSelector, child: &ComplexSelector) -> ComplexSelector {
    let has_nesting = child.head.nesting || child.rest.iter().any(|(_, c)| c.nesting);
    if !has_nesting {
        // Implicit descendant combinator: `parent child`
        let mut combined = parent.clone();
        combined
            .rest
            .push((Combinator::Descendant, child.head.clone()));
        combined.rest.extend(child.rest.iter().cloned());
        combined.span = child.span;
        return combined;
    }

    // Replace `&` in `child.head` (most common case: `&:hover`, `&.active`, `& > .x`)
    // or in a subsequent compound (`body &`).
    let mut result_head: Option<CompoundSelector> = None;
    let mut result_rest: Vec<(Combinator, CompoundSelector)> = Vec::new();

    let push_compound = |comb: Option<Combinator>,
                         comp: &CompoundSelector,
                         head: &mut Option<CompoundSelector>,
                         rest: &mut Vec<(Combinator, CompoundSelector)>| {
        if comp.nesting {
            let mut merged_tail = comp.clone();
            merged_tail.nesting = false;
            if parent.rest.is_empty() {
                let merged = merge_compounds(&parent.head, &merged_tail);
                match comb {
                    None => *head = Some(merged),
                    Some(c) => rest.push((c, merged)),
                }
            } else {
                match comb {
                    None => *head = Some(parent.head.clone()),
                    Some(c) => rest.push((c, parent.head.clone())),
                }
                for (i, (pc, pcomp)) in parent.rest.iter().enumerate() {
                    if i + 1 == parent.rest.len() {
                        rest.push((*pc, merge_compounds(pcomp, &merged_tail)));
                    } else {
                        rest.push((*pc, pcomp.clone()));
                    }
                }
            }
        } else {
            match comb {
                None => *head = Some(comp.clone()),
                Some(c) => rest.push((c, comp.clone())),
            }
        }
    };

    push_compound(None, &child.head, &mut result_head, &mut result_rest);
    for (c, comp) in &child.rest {
        push_compound(Some(*c), comp, &mut result_head, &mut result_rest);
    }

    ComplexSelector {
        head: result_head.unwrap_or_else(|| parent.head.clone()),
        rest: result_rest,
        span: child.span,
    }
}

fn merge_compounds(base: &CompoundSelector, extra: &CompoundSelector) -> CompoundSelector {
    let mut out = base.clone();
    if out.type_sel.is_none() {
        out.type_sel = extra.type_sel.clone();
    }
    if let Some(id) = &extra.id {
        if out.id.is_none() {
            out.id = Some(id.clone());
        } else {
            out.extra_ids.push(id.clone());
        }
    }
    out.extra_ids.extend(extra.extra_ids.iter().cloned());
    out.classes.extend(extra.classes.iter().cloned());
    out.attrs.extend(extra.attrs.iter().cloned());
    out.pseudos.extend(extra.pseudos.iter().cloned());
    if extra.pseudo_element.is_some() {
        out.pseudo_element = extra.pseudo_element.clone();
    }
    out
}

// ============================================================================
// Document-root selector matching (for root_var_scope)
// ============================================================================

fn selector_matches_document_root(sel: &ComplexSelector, theme: &str) -> bool {
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
    // Type selector may be absent, `*`, `html`, or `body`.
    if let Some(ts) = &comp.type_sel {
        match ts {
            TypeSelector::Universal => {}
            TypeSelector::Tag(tag)
                if tag.eq_ignore_ascii_case("html") || tag.eq_ignore_ascii_case("body") => {}
            _ => return false,
        }
    }
    // Pseudo-classes may only be `:root`.
    for p in &comp.pseudos {
        if !matches!(p, PseudoClass::Root) {
            return false;
        }
    }
    // Attributes: only `data-theme` (presence or exact match against `theme`).
    for attr in &comp.attrs {
        if !attr.name.eq_ignore_ascii_case("data-theme") {
            return false;
        }
        if let Some(op) = attr.op {
            if op != AttrOperator::Equals {
                return false;
            }
            let Some(val) = &attr.value else {
                return false;
            };
            if !val.eq_ignore_ascii_case(theme.trim()) {
                return false;
            }
        }
    }
    // Must actually target root (not an empty compound).
    !comp.is_empty()
}
