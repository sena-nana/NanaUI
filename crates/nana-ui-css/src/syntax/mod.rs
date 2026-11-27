//! Strict W3C CSS Syntax Level 3 foundation for NanaUI's CSS engine.
//!
//! Modules:
//! - [`span`]: exact UTF-8 byte spans & 1-based line/column mapping.
//! - [`diag`]: tiered structured diagnostics (`Error` vs `Warning`).
//! - [`tokenizer`]: UTF-8 char-safe W3C CSS Syntax Level 3 lexer.
//! - [`parser`]: nesting-bounded `ComponentValue`, stylesheet, rule, declaration,
//!   and CSS Nesting style-block parsers + slice combinators.
//! - [`vars`]: hierarchical token-stream `VarScope` and direct `ComponentValue`
//!   `var(...)` expansion with cycle/depth protection.
//! - [`selector`]: W3C Selectors Level 4 AST, quote-aware attributes, `An+B`,
//!   separate AND-composed `:is()`/`:where()`/`:not()`, and exact `Specificity`.
//! - [`at_rule`]: unified `@layer`, `@keyframes`, `@container`, `@media`
//!   (Kleene 3-valued), `@supports`, and `@font-face` parsers.
//! - [`values`]: unified angles, lengths, `calc()`/`min()`/`max()`/`clamp()`,
//!   `ThemeExpr`, colors, 1/2/3/4-value positions, and gradient stop fixup.

/// Maximum nesting depth permitted across blocks (`{}`, `[]`, `()`), functions
/// (`calc()`, `color-mix()`, `var()`, `:is()`), unary `+/-` chains, and boolean
/// conditions in `@media` / `@container` / `@supports`.
pub const MAX_NESTING: usize = 64;

pub mod at_rule;
pub mod diag;
pub mod parser;
pub mod selector;
pub mod span;
pub mod tokenizer;
pub mod values;
pub mod vars;

#[cfg(test)]
mod tests;

pub use at_rule::{
    CompareOp, ContainerCondition, ContainerFeatureName, ContainerRule, ContainerSizeQuery,
    ContainerValue, FontDisplay, FontFaceRule, FontFaceSrc, FontFaceSrcKind, FontFaceStyle,
    FontStretchRange, FontWeightRange, KeyframeBlock, KeyframeStop, KeyframesRule, Kleene3,
    LayerName, LayerOrder, LayerPrelude, LayerRegistry, MediaCondition, MediaFeature,
    MediaModifier, MediaQuery, MediaQueryList, MediaType, SupportsCondition, UnicodeRange,
    compare_layer_cascade, parse_container_prelude, parse_container_rule, parse_font_face_rule,
    parse_keyframes_rule, parse_layer_prelude, parse_media_query_list, parse_supports_condition,
};
pub use diag::{
    CssDiag, DiagCode, DiagSeverity, diagnose_unknown_property, is_ignorable_vendor_property,
};
pub use parser::{
    AtRule, BlockKind, ComponentValue, Declaration, QualifiedRule, Rule, StyleBlockItem,
    Stylesheet, expect_eof, parse_component_values, parse_declaration_list,
    parse_declarations_from_values, parse_rule_list, parse_style_block, parse_stylesheet,
    serialize_component_values, span_of_slice, split_comma_groups, split_whitespace_items, trim_ws,
};
pub use selector::{
    AnPlusB, AttrCase, AttrOperator, AttrSelector, Combinator, ComplexSelector, CompoundSelector,
    PseudoClass, PseudoElement, SelectorList, Specificity, TypeSelector, parse_an_plus_b,
    parse_selector_list,
};
pub use span::Span;
pub use tokenizer::{SpannedToken, Token, Tokenizer, tokenize};
pub use values::{
    ColorInterpolationMethod, ColorParseOptions, GradientStopPosition, HueInterpolationMethod,
    ParsedColor, ParsedGradientStop, normalize_gradient_stops, parse_angle_deg,
    parse_angle_deg_or_zero, parse_angle_token_deg, parse_calc_expr, parse_calc_expr_any,
    parse_color, parse_color_str, parse_gradient_stop, parse_hue_token_deg, parse_length_spec,
    parse_length_token, parse_named_color, parse_position, parse_size_length_spec,
    parse_theme_expr, parse_theme_token_name,
};
pub use vars::{MAX_EXPANDED_TOKENS, VarCascadeKey, VarExpandError, VarScope, expand_vars};
