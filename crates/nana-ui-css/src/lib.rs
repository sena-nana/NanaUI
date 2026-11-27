//! NanaUI's single CSS engine: stylesheet parsing, cascade, property lowering
//! onto [`nana_ui_core::StyleOp`], and exhaustive Rust patch codegen.
//!
//! Built strictly on [`syntax`] (CSS Syntax Level 3 tokenizer/parser, Selectors 4,
//! Variables 1, Values 4, At-Rules) and [`nana_ui_core::StyleOp`], with zero
//! external crate dependencies and zero thread-local state.
//!
//! Build-time compilers (`nana-ui-view-codegen`, `nana-ui-sfc`, `css!`, `view!`)
//! run this crate at compile time and emit pure Rust [`nana_ui_core`] data so
//! CSS parsing never enters `nana-ui-core`, `nana-ui-runtime`, `nana-ui-scene`,
//! or `nana-ui`.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

pub mod cascade;
pub mod codegen;
pub mod lower;
pub mod properties;
pub mod syntax;

#[cfg(test)]
mod tests;

pub use cascade::{
    CascadeKey, CustomPropDecl, ParsedRule, ParsedSheet, parse_stylesheet,
    parse_stylesheet_with_base,
};
pub use codegen::{
    emit_color_filter_rust, emit_font_face_rule_rust, emit_media_query_list_rust,
    emit_media_query_rust, emit_paint_color_rust, emit_paint_transform_rust, emit_style_op_rust,
    emit_style_ops_rust,
};
pub use lower::{
    ContainerAxis, ContainerInterval, ContainerPlan, ContainerPlanUnsupported, ContainerQuery,
    ContainerQueryUnsupported, MAX_CONTAINER_BREAKPOINTS, lower_container_query,
    lower_font_face_rule, lower_media_condition, lower_media_query, lower_media_query_list,
    plan_container_queries,
};
pub use properties::grid::{GridTrackListParse, parse_grid_track_list};
pub use properties::motion::{
    CompiledAnimation, CompiledTransition, css_ms_duration, parse_time_ms, parse_timing_function,
};
pub use properties::paint::{
    parse_color_filter_values, parse_linear_gradient, parse_radial_gradient,
};
pub use properties::pseudo::merge_scrollbar_skin;
pub use properties::transform::{ParsedTransform, parse_transform_values};
pub use properties::{
    CompileContext, CompiledDecls, CompiledInline, apply_css_text, apply_css_text_with,
    apply_style_ops, compile_declarations, compile_inline_css, is_supported_declaration,
};
pub use syntax::{
    AnPlusB, AttrCase, AttrOperator, AttrSelector, BlockKind, ColorParseOptions, Combinator,
    ComplexSelector, ComponentValue, CompoundSelector, CssDiag, Declaration, DiagCode,
    DiagSeverity, KeyframeBlock, KeyframeStop, KeyframesRule, LayerOrder, LayerRegistry,
    ParsedColor, PseudoClass, PseudoElement, SelectorList, Span, Specificity, TypeSelector,
    VarCascadeKey, VarScope, expand_vars, parse_color, parse_color_str, parse_length_spec,
    parse_theme_expr,
};
