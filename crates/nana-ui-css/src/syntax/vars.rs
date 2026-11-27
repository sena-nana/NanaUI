//! Token-stream CSS custom property scoping ([`VarScope`]) and `var(...)`
//! expansion directly on [`ComponentValue`] trees (zero string re-serialization).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use super::MAX_NESTING;
use super::at_rule::{LayerOrder, compare_layer_cascade};
use super::diag::{CssDiag, DiagCode};
use super::parser::{ComponentValue, trim_ws};
use super::selector::Specificity;
use super::span::Span;
use super::tokenizer::{SpannedToken, Token};

/// Hard ceiling on expanded token count per declaration to prevent exponential
/// ("billion laughs") custom property blowup.
pub const MAX_EXPANDED_TOKENS: usize = 16_384;

/// Cascade priority key for a custom property declaration inside a [`VarScope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarCascadeKey {
    pub important: bool,
    pub layer: LayerOrder,
    pub specificity: Specificity,
    pub source_order: u32,
}

impl VarCascadeKey {
    /// Default priority for direct / unlayered insertions with a given order.
    pub const fn unlayered(source_order: u32) -> Self {
        Self {
            important: false,
            layer: LayerOrder::UNLAYERED,
            specificity: Specificity::ZERO,
            source_order,
        }
    }

    /// Compare two cascade keys according to CSS Cascade 5:
    /// 1. Normal layered < Normal unlayered < `!important` unlayered < `!important` layered (reversed)
    /// 2. Higher specificity wins
    /// 3. Later source order wins
    pub fn compare(&self, other: &Self) -> Ordering {
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

impl Ord for VarCascadeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl PartialOrd for VarCascadeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct VarEntry {
    value: Vec<ComponentValue>,
    priority: VarCascadeKey,
}

/// Hierarchical token-stream scope of `--*` custom properties.
///
/// Child scopes inherit from an optional parent (`:root` -> ancestor -> element)
/// while local declarations obey CSS Cascade 5 layer / specificity / `!important`
/// precedence.
#[derive(Debug, Clone, Default)]
pub struct VarScope {
    parent: Option<Arc<VarScope>>,
    entries: HashMap<String, VarEntry>,
    next_order: u32,
}

impl VarScope {
    /// Create an empty root scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a child scope inheriting from `parent`.
    pub fn with_parent(parent: Arc<VarScope>) -> Self {
        Self {
            parent: Some(parent),
            entries: HashMap::new(),
            next_order: 0,
        }
    }

    /// Returns `true` if neither this scope nor any ancestor contains variables.
    pub fn is_empty(&self) -> bool {
        if !self.entries.is_empty() {
            return false;
        }
        self.parent.as_ref().is_none_or(|p| p.is_empty())
    }

    /// Insert or overwrite a custom property with an explicit [`VarCascadeKey`].
    ///
    /// An existing declaration in the same scope is only replaced if
    /// `priority >= existing.priority`. Non-`--` names are ignored.
    pub fn set_with_cascade(
        &mut self,
        name: impl Into<String>,
        value: Vec<ComponentValue>,
        priority: VarCascadeKey,
    ) {
        let name = name.into();
        if !name.starts_with("--") {
            return;
        }
        if let Some(existing) = self.entries.get(&name)
            && priority < existing.priority
        {
            return;
        }
        self.entries.insert(name, VarEntry { value, priority });
    }

    /// Convenience setter using monotonic unlayered source order.
    pub fn set(&mut self, name: impl Into<String>, value: Vec<ComponentValue>) {
        let order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        self.set_with_cascade(name, value, VarCascadeKey::unlayered(order));
    }

    /// Look up `name` in this scope, falling back up the parent chain.
    pub fn get(&self, name: &str) -> Option<&[ComponentValue]> {
        if let Some(entry) = self.entries.get(name) {
            return Some(entry.value.as_slice());
        }
        let mut cursor = self.parent.as_deref();
        while let Some(scope) = cursor {
            if let Some(entry) = scope.entries.get(name) {
                return Some(entry.value.as_slice());
            }
            cursor = scope.parent.as_deref();
        }
        None
    }

    /// Iterate local entries in this scope (excluding ancestors).
    pub fn local_entries(&self) -> impl Iterator<Item = (&str, &[ComponentValue])> {
        self.entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.value.as_slice()))
    }
}

/// Failure modes of [`expand_vars`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarExpandError {
    Cycle { name: String, span: Span },
    DepthExceeded { span: Span },
    TokenBudgetExceeded { span: Span },
    InvalidVarSyntax { span: Span, message: String },
    MissingVariable { name: String, span: Span },
}

/// Expand all `var(...)` calls directly on a [`ComponentValue`] slice without
/// serializing back to strings.
///
/// Behavior:
/// - Recursively expands inside nested functions (`calc(...)`, `color-mix(...)`)
///   and simple blocks (`(...)`, `[...]`, `{...}`).
/// - Supports arbitrarily nested fallbacks `var(--a, var(--b, 100px))` for all
///   properties, regardless of `preserve_nana_tokens`.
/// - When `preserve_nana_tokens` is `true`, references to `--nana-*` that are
///   not overridden in `scope` remain as `var(--nana-...)` function nodes
///   (while any nested `var(...)` inside their fallback is still expanded).
/// - Detects reference cycles (`--a` -> `--b` -> `--a`), nesting depth over
///   [`MAX_NESTING`], and exponential expansion over [`MAX_EXPANDED_TOKENS`].
pub fn expand_vars(
    values: &[ComponentValue],
    scope: &VarScope,
    preserve_nana_tokens: bool,
    diags: &mut Vec<CssDiag>,
) -> Result<Vec<ComponentValue>, VarExpandError> {
    let mut ctx = ExpandCtx {
        scope,
        preserve_nana_tokens,
        diags,
        active: Vec::new(),
        emitted: 0,
    };
    ctx.expand_slice(values, 0)
}

struct ExpandCtx<'a> {
    scope: &'a VarScope,
    preserve_nana_tokens: bool,
    diags: &'a mut Vec<CssDiag>,
    active: Vec<String>,
    emitted: usize,
}

impl ExpandCtx<'_> {
    fn bump_budget(&mut self, span: Span) -> Result<(), VarExpandError> {
        self.emitted += 1;
        if self.emitted > MAX_EXPANDED_TOKENS {
            self.diags.push(CssDiag::error(
                span,
                DiagCode::VarDepthExceeded,
                format!("var() expansion exceeded token budget ({MAX_EXPANDED_TOKENS})"),
            ));
            return Err(VarExpandError::TokenBudgetExceeded { span });
        }
        Ok(())
    }

    fn expand_slice(
        &mut self,
        values: &[ComponentValue],
        depth: usize,
    ) -> Result<Vec<ComponentValue>, VarExpandError> {
        if depth > MAX_NESTING {
            let span = values.first().map_or(Span::EMPTY, ComponentValue::span);
            self.diags.push(CssDiag::error(
                span,
                DiagCode::VarDepthExceeded,
                format!("var() expansion exceeded nesting limit ({MAX_NESTING})"),
            ));
            return Err(VarExpandError::DepthExceeded { span });
        }

        let mut out = Vec::with_capacity(values.len());
        for cv in values {
            match cv {
                ComponentValue::Token(st) => {
                    self.bump_budget(st.span)?;
                    out.push(ComponentValue::Token(st.clone()));
                }
                ComponentValue::SimpleBlock { kind, span, values } => {
                    self.bump_budget(*span)?;
                    let expanded = self.expand_slice(values, depth + 1)?;
                    out.push(ComponentValue::SimpleBlock {
                        kind: *kind,
                        span: *span,
                        values: expanded,
                    });
                }
                ComponentValue::Function { name, span, args } => {
                    if name.eq_ignore_ascii_case("var") {
                        let expanded = self.expand_var_fn(*span, args, depth)?;
                        out.extend(expanded);
                    } else {
                        self.bump_budget(*span)?;
                        let expanded_args = self.expand_slice(args, depth + 1)?;
                        out.push(ComponentValue::Function {
                            name: name.clone(),
                            span: *span,
                            args: expanded_args,
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    fn expand_var_fn(
        &mut self,
        span: Span,
        args: &[ComponentValue],
        depth: usize,
    ) -> Result<Vec<ComponentValue>, VarExpandError> {
        if depth >= MAX_NESTING {
            self.diags.push(CssDiag::error(
                span,
                DiagCode::VarDepthExceeded,
                format!("var() expansion exceeded nesting limit ({MAX_NESTING})"),
            ));
            return Err(VarExpandError::DepthExceeded { span });
        }

        // Split on the FIRST top-level comma so fallbacks may contain commas
        // (e.g. `var(--font, "Inter", sans-serif)`).
        let (head, fallback) = split_first_comma(args);
        let head = trim_ws(head);
        let [
            ComponentValue::Token(SpannedToken {
                token: Token::Ident(var_name),
                ..
            }),
        ] = head
        else {
            let msg = "var() first argument must be a single custom property name (`--*`)";
            self.diags
                .push(CssDiag::error(span, DiagCode::InvalidVarSyntax, msg));
            return Err(VarExpandError::InvalidVarSyntax {
                span,
                message: msg.to_string(),
            });
        };

        if !var_name.starts_with("--") || var_name.len() == 2 {
            let msg = format!("invalid custom property name `{var_name}` in var()");
            self.diags.push(CssDiag::error(
                span,
                DiagCode::InvalidVarSyntax,
                msg.clone(),
            ));
            return Err(VarExpandError::InvalidVarSyntax { span, message: msg });
        }

        // Cycle check.
        if self.active.iter().any(|n| n == var_name) {
            self.diags.push(CssDiag::error(
                span,
                DiagCode::VarCycle,
                format!("cyclic custom property reference involving `{var_name}`"),
            ));
            return Err(VarExpandError::Cycle {
                name: var_name.clone(),
                span,
            });
        }

        // Scope lookup first (allows explicit shadowing even of `--nana-*` if defined in scope).
        if let Some(bound) = self.scope.get(var_name) {
            let trimmed_bound = trim_ws(bound);
            // CSS Variables Level 1: `initial` on a custom property is the
            // guaranteed-invalid value, triggering fallback substitution.
            let is_guaranteed_invalid = matches!(
                trimmed_bound,
                [cv] if cv.as_ident().is_some_and(|id| id.eq_ignore_ascii_case("initial"))
            );
            if !is_guaranteed_invalid {
                self.active.push(var_name.clone());
                let result = self.expand_slice(bound, depth + 1);
                self.active.pop();
                return result;
            }
        }

        // Preserve unresolved `--nana-*` theme tokens when requested, while still
        // expanding any nested `var(...)` inside their fallback.
        if self.preserve_nana_tokens && var_name.starts_with("--nana-") {
            self.bump_budget(span)?;
            let mut preserved_args = Vec::new();
            preserved_args.push(head[0].clone());
            if let Some(fb) = fallback {
                preserved_args.push(ComponentValue::Token(SpannedToken::new(
                    Token::Comma,
                    Span::new(head[0].span().end, head[0].span().end),
                )));
                let expanded_fb = self.expand_slice(fb, depth + 1)?;
                preserved_args.extend(expanded_fb);
            }
            return Ok(vec![ComponentValue::Function {
                name: "var".to_string(),
                span,
                args: preserved_args,
            }]);
        }

        // Otherwise expand fallback if provided.
        if let Some(fb) = fallback {
            return self.expand_slice(trim_ws(fb), depth + 1);
        }

        // Undefined variable with no fallback -> guaranteed-invalid declaration per CSS Variables 1.
        self.diags.push(CssDiag::error(
            span,
            DiagCode::MissingVariable,
            format!("undefined CSS custom property `{var_name}` with no fallback"),
        ));
        Err(VarExpandError::MissingVariable {
            name: var_name.clone(),
            span,
        })
    }
}

fn split_first_comma(values: &[ComponentValue]) -> (&[ComponentValue], Option<&[ComponentValue]>) {
    for (i, cv) in values.iter().enumerate() {
        if matches!(cv.as_token(), Some(Token::Comma)) {
            return (&values[..i], Some(&values[i + 1..]));
        }
    }
    (values, None)
}
