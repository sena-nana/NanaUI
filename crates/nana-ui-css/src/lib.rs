//! NanaUI's CSS subset, in one place.
//!
//! A stylesheet is ordinary CSS within the subset `docs/reference/layout.md` lists:
//! selectors, the cascade, custom properties, `@media`, `@import`,
//! `@font-face`, `@keyframes` and the interactive pseudo-classes, mapped onto
//! the one Style Model ([`nana_ui_core::LayoutStyle`]). Two consumers share
//! it: the Vue path parses and cascades at run time (`nana-ui-vue`), and the
//! `.vue` compiler parses and matches at build time (`nana-ui-sfc`), so an
//! L3 view carries its styles as data and parses nothing while it runs.
//!
//! CSS parsing stays out of `nana-ui-core`, `nana-ui-runtime`,
//! `nana-ui-scene` and `nana-ui`.
// Carried over with the code from nana-ui-vue.
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

pub mod css_at_rule;
pub mod css_cascade;
pub mod css_font_face;
pub mod css_interactive;
pub mod css_map;
pub mod css_motion;
pub mod css_paint;
pub mod css_paint_transform;
pub mod css_written;
pub mod shell_contract;
pub mod style;

pub use css_at_rule::{
    FontFaceRule, FontFaceSrc, ImportPrelude, LayerPrelude, MAX_FONT_FACE_BYTES, MAX_IMPORT_DEPTH,
    MAX_REGISTERED_FONT_BYTES, MAX_STYLESHEET_BYTES, MediaEnvironment, MediaFeature, MediaQuery,
    MediaQueryList, MediaType, MemoryStylesheetLoader, PackagedStylesheetLoader,
    ParseStylesheetOptions, StylesheetLoader, evaluate_media_query, evaluate_media_query_list,
    evaluate_supports_condition, is_blocked_href, parse_import_prelude, parse_layer_prelude,
    parse_media_query_list,
};
pub use css_cascade::{
    AnPlusB, AttrCase, AttrOperator, AttrSelector, Combinator, CompoundSelector, DeclarationEntry,
    MatchContext, MatchNode, MatchSubtree, MediaEnv, OwnedMatchTree, Selector, SimpleCompound,
    Specificity, StyleRule, StylesheetParseReport, apply_stylesheet_to_layout,
    collect_document_custom_properties_from_rules, matched_declaration_entries,
    matched_declarations, parse_stylesheet, parse_stylesheet_full,
    parse_stylesheet_full_with_layers, parse_stylesheet_full_with_options,
    parse_stylesheet_with_report, rebuild_layout_style, selector_matches,
    stylesheet_needs_relative,
};
pub use css_font_face::{
    FontFaceSrcKind, FontFaceStyle, parse_font_face_at_rule, parse_font_face_rules,
};
pub use css_interactive::{
    GeneratedPseudo, GeneratedPseudoMatch, GeneratedPseudoRule, InteractiveMatchState,
    InteractivePseudo, InteractivePseudoFlags, InteractiveSelector, InteractiveStyleRule,
    KeyframeBlock, KeyframeSelector, KeyframesRule, MediaRule, MotionDeclarations, MotionStyleRule,
    ParsedStylesheet, ScrollbarPseudo, ScrollbarPseudoRule, keyframes_by_name,
    matched_generated_pseudo, matched_interactive_rules, matched_motion_rules,
    matched_scrollbar_pseudo, merge_parsed_stylesheet, partition_motion_entries,
};
pub use css_map::{
    AlignSpec, BoxSizing, CssLayoutParse, DirSpec, DisplaySpec, FlexDirection, FlexWrap,
    FontSizeContext, GridAutoFlow, GridTrack, GridTrackListParse, GridTrackListUnsupported,
    JustifySpec, LayoutStyle, LayoutStyleCss, LengthSpec, LineHeightSpec, OverflowSpec,
    PaddingSpec, ParentBox, PositionSpec, collect_document_css_custom_properties,
    parse_box_edge_length, parse_css_font_family, parse_css_font_feature_settings,
    parse_css_font_kerning, parse_css_font_size, parse_css_font_variation_settings,
    parse_css_font_weight, parse_css_length_px, parse_css_letter_spacing, parse_css_line_break,
    parse_css_line_height, parse_css_word_break, parse_grid_template_columns,
    parse_grid_track_list_result, parse_inset_length, resolve_grid_column_widths,
    resolve_grid_track_sizes, resolve_paint_color,
};
pub use css_written::{WrittenLayout, written_layout};
pub use style::{
    CssPaintColor, is_non_token_css_color, map_css_color_for_tokens, parse_css_color,
    parse_css_paint_color, resolve_css_paint_color,
};
