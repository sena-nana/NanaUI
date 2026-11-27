//! Exhaustive Rust source emitter for [`nana_ui_core::StyleOp`],
//! [`nana_ui_core::MediaQueryList`], and [`nana_ui_core::FontFaceRule`].
//!
//! Every [`StyleOp`] variant is matched explicitly (no wildcard arm), so
//! adding a new variant in `nana-ui-core` fails to compile here until codegen
//! handles it. Generated statements target the caller-provided `patch_root`
//! module (e.g. `"::nana_ui_runtime::view::__patch"`) and mutate a binding
//! named `__nana_layout: &mut LayoutStyle`.

use std::fmt::Write as _;
use std::sync::Arc;

use nana_ui_core::dynamic_layout::{
    AdaptationProfile, AxisElasticity, DiscreteAdaptation, DiscreteCandidate, ElasticFloor,
    ElasticLength, KeepWith, LayoutCost,
};
use nana_ui_core::shared::{Shared, SharedDefault};
use nana_ui_core::{
    AlignSpec, AnchorSpec, BackdropFilter, BackfaceVisibility, BackgroundImage, BackgroundImageFit,
    BackgroundPosition, BackgroundRepeat, BorderImageSlice, BorderImageSpec, BorderStyle,
    BorderWidth, BoxShadowSpec, BoxSizing, CalcBinOp, CalcExpr, CalcExprRef, ClipCircle,
    ClipEllipse, ClipInset, ClipPath, ClipPoint, ClipShapeRadius, ColorFilter, ContainerType,
    ControlPadding, ControlSize, CssGradient, CursorSpec, CustomCursorSpec, CustomCursorVariant,
    CustomCursorVector, DirSpec, DisplaySpec, DominantBaselineSpec, ElevationRole,
    FilterDropShadow, FlexDirection, FlexWrap, FontFaceRule, FontFaceSource, FontFeatureSetting,
    FontKerningSpec, FontStyleSpec, FontVariationSetting, GradientStop, GridAutoFlow, GridLine,
    GridRepeatAuto, GridTemplateAreas, GridTrack, GridTrackListUnsupported, HitTestBehavior,
    HyphensSpec, IconPathCommand, IconShape, ImageSampling, JustifySpec, LengthSpec, LineBreakSpec,
    LineHeightSpec, LineRole, LinearGradient, MaskImage, MaterialRef, MaterialTexture,
    MediaColorScheme, MediaComparison, MediaCondition, MediaContrast, MediaFeature, MediaHover,
    MediaOrientation, MediaPointer, MediaQuery, MediaQueryList, MediaReducedMotion, MediaType,
    MixBlendMode, OutlineStyle, OverflowSpec, OverflowWrapSpec, PaintColor, PaintMat4,
    PaintTransform, PointerEventsSpec, PopoverAlignment, PopoverPlacement, PositionSpec,
    RadialGradient, RadiusTier, ScrollEdgeFade, ScrollbarSkin, SemanticColorMix, SemanticColorRole,
    SpacingStep, StyleOp, SystemCursor, TextAlignSpec, TextAutospaceSpec, TextDecorationLine,
    TextEmphasisColor, TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle, TextJustifySpec,
    TextOrientationSpec, TextShadowSpec, TextSpacingTrimSpec, TextStrokeSpec, TextWeight,
    TextWrapStyleSpec, ThemeColorPairRef, ThemeExpr, ThemeToken, TransformBox, TransformOrigin,
    TypeRole, UserSelectSpec, VerticalAlignSpec, ViewportAxis, VisibilitySpec, WhiteSpaceSpec,
    WindowResizeEdge, WordBreakSpec, WritingModeSpec,
};

/// Emit a single [`StyleOp`] as a Rust statement invoking
/// `{patch_root}::set::<setter>(__nana_layout, <value>);`.
///
/// Exhaustive over all variants of [`StyleOp`].
pub fn emit_style_op_rust(op: &StyleOp, patch_root: &str, out: &mut String) {
    match op {
        // ---- top ----
        StyleOp::LayoutIsolation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Direction(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Dir(v) => emit_setter(op, v, patch_root, out),
        StyleOp::WritingMode(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FlexReverse(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Order(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FlexWrap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::EqualWrap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Display(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BoxSizing(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Position(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Anchor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ZIndex(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Isolation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Gap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::RowGap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ColumnGap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Padding(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PaddingTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PaddingRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PaddingBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PaddingLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Margin(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MarginTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MarginRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MarginBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MarginLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OffsetTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OffsetRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OffsetBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OffsetLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Width(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Height(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MinWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MaxWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MinHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MaxHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::AllowShrink(v) => emit_setter(op, v, patch_root, out),
        StyleOp::AlignItems(v) => emit_setter(op, v, patch_root, out),
        StyleOp::AlignSelf(v) => emit_setter(op, v, patch_root, out),
        StyleOp::AlignContent(v) => emit_setter(op, v, patch_root, out),
        StyleOp::JustifyContent(v) => emit_setter(op, v, patch_root, out),
        StyleOp::JustifyItems(v) => emit_setter(op, v, patch_root, out),
        StyleOp::JustifySelf(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FlexGrow(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FlexShrink(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FlexBasis(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Adaptation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OverflowX(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OverflowY(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextOverflowEllipsis(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PointerEvents(v) => emit_setter(op, v, patch_root, out),
        StyleOp::HitTest(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Inert(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Cursor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::UserSelect(v) => emit_setter(op, v, patch_root, out),
        StyleOp::WhiteSpace(v) => emit_setter(op, v, patch_root, out),
        StyleOp::AspectRatio(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextAlign(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontSize(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontWeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontItalic(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LineHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Hidden(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Opacity(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderRadius(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderStyle(v) => emit_setter(op, v, patch_root, out),

        // ---- theme ----
        StyleOp::ThemeGap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeRowGap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeColumnGap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemePadding(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemePaddingTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemePaddingRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemePaddingBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemePaddingLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMargin(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMarginTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMarginRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMarginBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMarginLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeOffsetTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeOffsetRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeOffsetBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeOffsetLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMinWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMaxWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMinHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeMaxHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeFlexBasis(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeFontSize(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeFontWeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeLineHeight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeLetterSpacing(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderTopWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderRightWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderBottomWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderLeftWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeBorderRadius(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ThemeScrollbarRoom(v) => emit_setter(op, v, patch_root, out),

        // ---- paint_colors ----
        StyleOp::Color(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Background(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderTopColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderRightColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderBottomColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderLeftColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OutlineColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::SelectionBackground(v) => emit_setter(op, v, patch_root, out),
        StyleOp::SelectionColor(v) => emit_setter(op, v, patch_root, out),

        // ---- transforms ----
        StyleOp::Transform(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Transform3d(v) => emit_setter(op, v, patch_root, out),
        StyleOp::UnsupportedTransform(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TransformOrigin(v) => emit_setter(op, v, patch_root, out),
        StyleOp::CssPerspective(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PerspectiveOrigin(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Preserve3d(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackfaceVisibility(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TransformBox(v) => emit_setter(op, v, patch_root, out),

        // ---- logical_edges ----
        StyleOp::LogicalPaddingInlineStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingInlineEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingBlockStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingBlockEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingPhysTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingPhysRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingPhysBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingPhysLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingGenerations(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingNextGen(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalPaddingDerived(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginInlineStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginInlineEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginBlockStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginBlockEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginPhysTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginPhysRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginPhysBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginPhysLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginGenerations(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginNextGen(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalMarginDerived(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetInlineStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetInlineEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetBlockStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetBlockEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetPhysTop(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetPhysRight(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetPhysBottom(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetPhysLeft(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetGenerations(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetNextGen(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LogicalInsetDerived(v) => emit_setter(op, v, patch_root, out),

        // ---- text_ext ----
        StyleOp::FontFamily(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LetterSpacing(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LineClamp(v) => emit_setter(op, v, patch_root, out),
        StyleOp::WordBreak(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OverflowWrap(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextDecoration(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextEmphasisStyle(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextEmphasisPosition(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextEmphasisColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontFeatures(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontVariationSettings(v) => emit_setter(op, v, patch_root, out),
        StyleOp::FontKerning(v) => emit_setter(op, v, patch_root, out),
        StyleOp::LineBreak(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Hyphens(v) => emit_setter(op, v, patch_root, out),
        StyleOp::UnsupportedFontVariation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::UnsupportedWritingMode(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextOrientation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::DominantBaseline(v) => emit_setter(op, v, patch_root, out),
        StyleOp::VerticalAlign(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PlaceholderColor(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PlaceholderOpacity(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextSpacingTrim(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextAutospace(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextJustify(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextWrapStyle(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextJustifyLines(v) => emit_setter(op, v, patch_root, out),

        // ---- grid ----
        StyleOp::GridColumns(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRows(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnsUnsupported(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowsUnsupported(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridAutoColumns(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridAutoRows(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridAutoFlow(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnsRepeat(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowsRepeat(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnsSubgrid(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowsSubgrid(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridTemplateAreas(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnLineNames(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowLineNames(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridColumnEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowStart(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridRowEnd(v) => emit_setter(op, v, patch_root, out),
        StyleOp::GridArea(v) => emit_setter(op, v, patch_root, out),

        // ---- border_sides ----
        StyleOp::BorderTopWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderRightWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderBottomWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderLeftWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderTopStyle(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderRightStyle(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderBottomStyle(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderLeftStyle(v) => emit_setter(op, v, patch_root, out),

        // ---- container ----
        StyleOp::ContainerType(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ContainerName(v) => emit_setter(op, v, patch_root, out),

        // ---- paint ----
        StyleOp::BackgroundMaterial(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderMaterial(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextMaterial(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Visibility(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderRadii(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BoxShadows(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Elevation(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Scrollbar(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ScrollEdgeFade(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextShadows(v) => emit_setter(op, v, patch_root, out),
        StyleOp::TextStroke(v) => emit_setter(op, v, patch_root, out),
        StyleOp::PaintOrderStrokeFirst(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MixBlend(v) => emit_setter(op, v, patch_root, out),
        StyleOp::WideFilterBlur(v) => emit_setter(op, v, patch_root, out),
        StyleOp::MotionShutter(v) => emit_setter(op, v, patch_root, out),
        StyleOp::UnsupportedBorderImage(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BorderImage(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackgroundLayers(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackgroundSizeList(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackgroundSizeLengths(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackgroundPositionList(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackgroundRepeatList(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ContentImage(v) => emit_setter(op, v, patch_root, out),
        StyleOp::SkippedReplaced(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ObjectFit(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ObjectPosition(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Mask(v) => emit_setter(op, v, patch_root, out),
        StyleOp::ClipPath(v) => emit_setter(op, v, patch_root, out),
        StyleOp::Filter(v) => emit_setter(op, v, patch_root, out),
        StyleOp::BackdropFilter(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OutlineWidth(v) => emit_setter(op, v, patch_root, out),
        StyleOp::OutlineStyle(v) => emit_setter(op, v, patch_root, out),
    }
}

/// Emit a slice of [`StyleOp`]s as concatenated Rust statements targeting
/// `__nana_layout: &mut LayoutStyle`.
pub fn emit_style_ops_rust(ops: &[StyleOp], patch_root: &str) -> String {
    let mut out = String::with_capacity(ops.len() * 64);
    for op in ops {
        emit_style_op_rust(op, patch_root, &mut out);
    }
    out
}

/// Emit a Rust expression for a [`PaintColor`] referencing types under `patch_root`.
pub fn emit_paint_color_rust(color: &PaintColor, patch_root: &str) -> String {
    let mut out = String::new();
    color.emit_rust(patch_root, &mut out);
    out
}

/// Emit a Rust expression for a [`ColorFilter`] referencing types under `patch_root`.
pub fn emit_color_filter_rust(filter: &ColorFilter, patch_root: &str) -> String {
    let mut out = String::new();
    filter.emit_rust(patch_root, &mut out);
    out
}

/// Emit a Rust expression for a [`PaintTransform`] referencing types under `patch_root`.
pub fn emit_paint_transform_rust(transform: &PaintTransform, patch_root: &str) -> String {
    let mut out = String::new();
    transform.emit_rust(patch_root, &mut out);
    out
}

/// Emit a Rust expression for a [`MediaQueryList`] referencing types under `core_root`
/// (e.g. `"::nana_ui_core"`).
pub fn emit_media_query_list_rust(list: &MediaQueryList, core_root: &str) -> String {
    let mut out = String::new();
    list.emit_rust(core_root, &mut out);
    out
}

/// Emit a Rust expression for a single [`MediaQuery`] referencing types under `core_root`.
pub fn emit_media_query_rust(query: &MediaQuery, core_root: &str) -> String {
    let mut out = String::new();
    query.emit_rust(core_root, &mut out);
    out
}

/// Emit a Rust expression for a [`FontFaceRule`] referencing types under `core_root`
/// (e.g. `"::nana_ui_core"`).
pub fn emit_font_face_rule_rust(rule: &FontFaceRule, core_root: &str) -> String {
    let mut out = String::new();
    rule.emit_rust(core_root, &mut out);
    out
}

fn emit_setter<T: EmitRust>(op: &StyleOp, value: &T, root: &str, out: &mut String) {
    out.push_str(root);
    out.push_str("::set::");
    out.push_str(op.setter_path());
    out.push_str("(__nana_layout,");
    value.emit_rust(root, out);
    out.push_str(");");
}

// ---------------------------------------------------------------------------
// EmitRust trait and implementations
// ---------------------------------------------------------------------------

trait EmitRust {
    fn emit_rust(&self, root: &str, out: &mut String);
}

fn emit_path(root: &str, suffix: &str, out: &mut String) {
    out.push_str(root);
    out.push_str("::");
    out.push_str(suffix);
}

fn emit_slice_items<T: EmitRust>(items: &[T], root: &str, out: &mut String) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        item.emit_rust(root, out);
    }
}

impl EmitRust for bool {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        out.push_str(if *self { "true" } else { "false" });
    }
}

impl EmitRust for u8 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self}u8");
    }
}

impl EmitRust for u16 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self}u16");
    }
}

impl EmitRust for u32 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self}u32");
    }
}

impl EmitRust for i16 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self}i16");
    }
}

impl EmitRust for i32 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self}i32");
    }
}

impl EmitRust for f32 {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        if self.is_nan() {
            out.push_str("f32::NAN");
        } else if *self == f32::INFINITY {
            out.push_str("f32::INFINITY");
        } else if *self == f32::NEG_INFINITY {
            out.push_str("f32::NEG_INFINITY");
        } else {
            let _ = write!(out, "{self:?}f32");
        }
    }
}

impl EmitRust for String {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let _ = write!(out, "{self:?}.to_string()");
    }
}

impl EmitRust for Arc<str> {
    fn emit_rust(&self, _root: &str, out: &mut String) {
        let s: &str = self;
        let _ = write!(out, "::std::sync::Arc::<str>::from({s:?})");
    }
}

impl<T: EmitRust> EmitRust for Option<T> {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Some(v) => {
                out.push_str("Some(");
                v.emit_rust(root, out);
                out.push(')');
            }
            None => out.push_str("None"),
        }
    }
}

impl<T: EmitRust> EmitRust for Vec<T> {
    fn emit_rust(&self, root: &str, out: &mut String) {
        if self.is_empty() {
            out.push_str("::std::vec::Vec::new()");
        } else {
            out.push_str("vec![");
            emit_slice_items(self, root, out);
            out.push(']');
        }
    }
}

impl<T: EmitRust, const N: usize> EmitRust for [T; N] {
    fn emit_rust(&self, root: &str, out: &mut String) {
        out.push('[');
        emit_slice_items(self, root, out);
        out.push(']');
    }
}

impl<A: EmitRust, B: EmitRust> EmitRust for (A, B) {
    fn emit_rust(&self, root: &str, out: &mut String) {
        out.push('(');
        self.0.emit_rust(root, out);
        out.push(',');
        self.1.emit_rust(root, out);
        out.push(')');
    }
}

impl<T: EmitRust> EmitRust for Arc<T> {
    fn emit_rust(&self, root: &str, out: &mut String) {
        out.push_str("::std::sync::Arc::new(");
        (**self).emit_rust(root, out);
        out.push(')');
    }
}

impl<T: EmitRust + SharedDefault> EmitRust for Shared<T> {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "Shared::new(", out);
        (**self).emit_rust(root, out);
        out.push(')');
    }
}

// ---- Simple C-like enums ----

macro_rules! emit_unit_enum {
    ($ty:ty, $name:literal, { $($variant:ident),* $(,)? }) => {
        impl EmitRust for $ty {
            fn emit_rust(&self, root: &str, out: &mut String) {
                let variant = match self {
                    $(Self::$variant => stringify!($variant),)*
                };
                emit_path(root, $name, out);
                out.push_str("::");
                out.push_str(variant);
            }
        }
    };
}

emit_unit_enum!(FlexDirection, "FlexDirection", { Column, Row });
emit_unit_enum!(DirSpec, "DirSpec", { Ltr, Rtl });
emit_unit_enum!(WritingModeSpec, "WritingModeSpec", { HorizontalTb, VerticalRl, VerticalLr });
emit_unit_enum!(FlexWrap, "FlexWrap", { NoWrap, Wrap, WrapReverse });
emit_unit_enum!(DisplaySpec, "DisplaySpec", { None, Flex, InlineFlex, Grid, InlineGrid, Inline, Layers, Free });
emit_unit_enum!(BoxSizing, "BoxSizing", { BorderBox, ContentBox });
emit_unit_enum!(PositionSpec, "PositionSpec", { Static, Relative, Absolute, Fixed, Sticky });
emit_unit_enum!(PopoverPlacement, "PopoverPlacement", { Top, Bottom, Left, Right });
emit_unit_enum!(PopoverAlignment, "PopoverAlignment", { Start, Center, End });
emit_unit_enum!(AlignSpec, "AlignSpec", { Start, Center, End, Stretch, Baseline });
emit_unit_enum!(JustifySpec, "JustifySpec", { Start, Center, End, SpaceBetween, SpaceAround, SpaceEvenly, Stretch });
emit_unit_enum!(OverflowSpec, "OverflowSpec", { Visible, Hidden, Auto, Scroll });
emit_unit_enum!(PointerEventsSpec, "PointerEventsSpec", { Auto, None });
emit_unit_enum!(WindowResizeEdge, "WindowResizeEdge", { North, South, East, West, NorthEast, NorthWest, SouthEast, SouthWest });
emit_unit_enum!(SystemCursor, "SystemCursor", { Default, Pointer, Text, Move, Grab, Grabbing, NotAllowed, Crosshair, EwResize, NsResize, Help, Wait, Progress, ZoomIn, ZoomOut, None });
emit_unit_enum!(UserSelectSpec, "UserSelectSpec", { Auto, Text, All, Contain, None });
emit_unit_enum!(WhiteSpaceSpec, "WhiteSpaceSpec", { Normal, Nowrap, Pre, PreWrap, PreLine });
emit_unit_enum!(TextAlignSpec, "TextAlignSpec", { Start, Center, End, Left, Right, Justify });
emit_unit_enum!(BorderStyle, "BorderStyle", { None, Solid, Dashed, Dotted, Unsupported });
emit_unit_enum!(OutlineStyle, "OutlineStyle", { None, Solid });
emit_unit_enum!(VisibilitySpec, "VisibilitySpec", { Visible, Hidden });
emit_unit_enum!(BackfaceVisibility, "BackfaceVisibility", { Visible, Hidden });
emit_unit_enum!(TransformBox, "TransformBox", { ViewBox, BorderBox, FillBox, ContentBox });
emit_unit_enum!(WordBreakSpec, "WordBreakSpec", { Normal, BreakAll, BreakWord });
emit_unit_enum!(OverflowWrapSpec, "OverflowWrapSpec", { Normal, Anywhere });
emit_unit_enum!(TextEmphasisShape, "TextEmphasisShape", { Dot, Circle, DoubleCircle, Triangle, Sesame });
emit_unit_enum!(FontKerningSpec, "FontKerningSpec", { Auto, Normal, None });
emit_unit_enum!(LineBreakSpec, "LineBreakSpec", { Auto, Normal, Anywhere, Strict, Loose });
emit_unit_enum!(HyphensSpec, "HyphensSpec", { None, Manual, Auto });
emit_unit_enum!(TextOrientationSpec, "TextOrientationSpec", { Mixed, Upright, Sideways });
emit_unit_enum!(DominantBaselineSpec, "DominantBaselineSpec", { Auto, Central, Alphabetic, Ideographic });
emit_unit_enum!(TextSpacingTrimSpec, "TextSpacingTrimSpec", { SpaceAll, Normal, TrimBoth, Auto });
emit_unit_enum!(TextAutospaceSpec, "TextAutospaceSpec", { NoAutospace, Normal });
emit_unit_enum!(TextJustifySpec, "TextJustifySpec", { Auto, None, InterWord, InterCharacter });
emit_unit_enum!(TextWrapStyleSpec, "TextWrapStyleSpec", { Auto, Pretty });
emit_unit_enum!(GridTrackListUnsupported, "GridTrackListUnsupported", { RepeatAutoFit, RepeatAutoFill, NestedRepeat, Subgrid });
emit_unit_enum!(GridAutoFlow, "GridAutoFlow", { Row, Column, RowDense, ColumnDense });
emit_unit_enum!(ContainerType, "ContainerType", { Normal, InlineSize, Size });
emit_unit_enum!(ElevationRole, "ElevationRole", { Surface, Overlay, Menu, Dropdown, MenuBar });
emit_unit_enum!(MixBlendMode, "MixBlendMode", {
    Normal, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight,
    SoftLight, Difference, Exclusion, Hue, Saturation, Color, Luminosity, SourceIn, SourceOut,
    SourceAtop, DestinationOver, DestinationIn, DestinationOut, DestinationAtop, Xor, Copy, Lighter
});
emit_unit_enum!(BackgroundImageFit, "BackgroundImageFit", { Cover, Contain, Stretch, Auto, Length, ScaleDown });
emit_unit_enum!(BackgroundRepeat, "BackgroundRepeat", { NoRepeat, Repeat, RepeatX, RepeatY, Round, RoundX, RoundY, Unsupported });
emit_unit_enum!(ImageSampling, "ImageSampling", { Resample, Mipmap, Nearest });
emit_unit_enum!(ViewportAxis, "ViewportAxis", { Width, Height, Min, Max });
emit_unit_enum!(CalcBinOp, "CalcBinOp", { Add, Sub, Mul, Div });
emit_unit_enum!(KeepWith, "KeepWith", { None, Previous, Next });
emit_unit_enum!(SpacingStep, "SpacingStep", { Xxs, Xs, Sm, Md, Lg, Xl, Xxl, Xxxl, PageTight, Page });
emit_unit_enum!(TypeRole, "TypeRole", { Hint, Meta, Body, Section, CardTitle, Heading, Title, Display });
emit_unit_enum!(TextWeight, "TextWeight", { Regular, Medium, Semibold, Bold });
emit_unit_enum!(LineRole, "LineRole", { Line, Tall });
emit_unit_enum!(BorderWidth, "BorderWidth", { Hairline });
emit_unit_enum!(RadiusTier, "RadiusTier", { Xs, Sm, Md, Lg, Xl, Xxl, Full });
emit_unit_enum!(ControlSize, "ControlSize", { Small, Medium, Large });
emit_unit_enum!(ControlPadding, "ControlPadding", { Compact, Standard, Roomy, Field, ListItem });
emit_unit_enum!(SemanticColorRole, "SemanticColorRole", {
    Background, Surface, Subtle, Raised, Header, Hover, Active, Selected, SelectedHover,
    SelectedPressed, Border, BorderSoft, BorderStrong, Text, Muted, Faint, Accent, AccentStrong,
    AccentSoft, AccentSoftHover, AccentSoftPressed, AccentOnSoft, AccentText, FocusSurface,
    FocusBorder, FocusText, Highlight, HighlightHover, HighlightPressed, HighlightText, Success,
    Warning, WarningSoft, WarningSoftHover, WarningSoftPressed, Danger, DangerSoftHover,
    DangerSoftPressed, Titlebar, OnMedia, ModalScrim, Function, Builtin, Type, Variable,
    Parameter, Const, Texture, Property, Keyword
});
emit_unit_enum!(MediaType, "MediaType", { All, Screen, Print, Other });
emit_unit_enum!(MediaComparison, "MediaComparison", { Lt, Le, Eq, Ge, Gt });
emit_unit_enum!(MediaOrientation, "MediaOrientation", { Portrait, Landscape });
emit_unit_enum!(MediaColorScheme, "MediaColorScheme", { Light, Dark });
emit_unit_enum!(MediaReducedMotion, "MediaReducedMotion", { NoPreference, Reduce });
emit_unit_enum!(MediaContrast, "MediaContrast", { NoPreference, More, Less, Custom });
emit_unit_enum!(MediaHover, "MediaHover", { None, Hover });
emit_unit_enum!(MediaPointer, "MediaPointer", { None, Coarse, Fine });
emit_unit_enum!(FontStyleSpec, "FontStyleSpec", { Normal, Italic, Oblique });

// ---- Compound types ----

impl EmitRust for LengthSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Px(v) => {
                emit_path(root, "LengthSpec::Px(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Percent(v) => {
                emit_path(root, "LengthSpec::Percent(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Em(v) => {
                emit_path(root, "LengthSpec::Em(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Rem(v) => {
                emit_path(root, "LengthSpec::Rem(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Viewport { axis, value } => {
                emit_path(root, "LengthSpec::Viewport{axis: ", out);
                axis.emit_rust(root, out);
                out.push_str(",value: ");
                value.emit_rust(root, out);
                out.push('}');
            }
            Self::PercentOffset { offset_q7, percent } => {
                emit_path(root, "LengthSpec::PercentOffset{offset_q7: ", out);
                offset_q7.emit_rust(root, out);
                out.push_str(",percent: ");
                percent.emit_rust(root, out);
                out.push('}');
            }
            Self::Calc(expr) => {
                emit_path(root, "LengthSpec::Calc(", out);
                expr.emit_rust(root, out);
                out.push(')');
            }
            Self::Fill => emit_path(root, "LengthSpec::Fill", out),
            Self::Shrink => emit_path(root, "LengthSpec::Shrink", out),
            Self::Auto => emit_path(root, "LengthSpec::Auto", out),
            Self::MinContent => emit_path(root, "LengthSpec::MinContent", out),
            Self::MaxContent => emit_path(root, "LengthSpec::MaxContent", out),
            Self::FitContent => emit_path(root, "LengthSpec::FitContent", out),
        }
    }
}

impl EmitRust for CalcExprRef {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "intern_calc(::std::boxed::Box::new(", out);
        self.inner().emit_rust(root, out);
        out.push_str("))");
    }
}

impl EmitRust for CalcExpr {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Number(v) => {
                emit_path(root, "CalcExpr::Number(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Px(v) => {
                emit_path(root, "CalcExpr::Px(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Percent(v) => {
                emit_path(root, "CalcExpr::Percent(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Em(v) => {
                emit_path(root, "CalcExpr::Em(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Rem(v) => {
                emit_path(root, "CalcExpr::Rem(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Viewport { axis, value } => {
                emit_path(root, "CalcExpr::Viewport{axis: ", out);
                axis.emit_rust(root, out);
                out.push_str(",value: ");
                value.emit_rust(root, out);
                out.push('}');
            }
            Self::Binary { op, left, right } => {
                emit_path(root, "CalcExpr::Binary{op: ", out);
                op.emit_rust(root, out);
                out.push_str(",left: ::std::boxed::Box::new(");
                left.emit_rust(root, out);
                out.push_str("),right: ::std::boxed::Box::new(");
                right.emit_rust(root, out);
                out.push_str(")}");
            }
            Self::Neg(inner) => {
                emit_path(root, "CalcExpr::Neg(::std::boxed::Box::new(", out);
                inner.emit_rust(root, out);
                out.push_str("))");
            }
            Self::Min(a, b) => {
                emit_path(root, "CalcExpr::Min(::std::boxed::Box::new(", out);
                a.emit_rust(root, out);
                out.push_str("),::std::boxed::Box::new(");
                b.emit_rust(root, out);
                out.push_str("))");
            }
            Self::Max(a, b) => {
                emit_path(root, "CalcExpr::Max(::std::boxed::Box::new(", out);
                a.emit_rust(root, out);
                out.push_str("),::std::boxed::Box::new(");
                b.emit_rust(root, out);
                out.push_str("))");
            }
            Self::Clamp { min, val, max } => {
                emit_path(root, "CalcExpr::Clamp{min: ::std::boxed::Box::new(", out);
                min.emit_rust(root, out);
                out.push_str("),val: ::std::boxed::Box::new(");
                val.emit_rust(root, out);
                out.push_str("),max: ::std::boxed::Box::new(");
                max.emit_rust(root, out);
                out.push_str(")}");
            }
        }
    }
}

impl EmitRust for LineHeightSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Relative(v) => {
                emit_path(root, "LineHeightSpec::Relative(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Absolute(v) => {
                emit_path(root, "LineHeightSpec::Absolute(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for VerticalAlignSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Baseline => emit_path(root, "VerticalAlignSpec::Baseline", out),
            Self::Sub => emit_path(root, "VerticalAlignSpec::Sub", out),
            Self::Super => emit_path(root, "VerticalAlignSpec::Super", out),
            Self::TextTop => emit_path(root, "VerticalAlignSpec::TextTop", out),
            Self::TextBottom => emit_path(root, "VerticalAlignSpec::TextBottom", out),
            Self::Middle => emit_path(root, "VerticalAlignSpec::Middle", out),
            Self::Top => emit_path(root, "VerticalAlignSpec::Top", out),
            Self::Bottom => emit_path(root, "VerticalAlignSpec::Bottom", out),
            Self::Offset(len) => {
                emit_path(root, "VerticalAlignSpec::Offset(", out);
                len.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for HitTestBehavior {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Auto => emit_path(root, "HitTestBehavior::Auto", out),
            Self::Client => emit_path(root, "HitTestBehavior::Client", out),
            Self::PassThrough => emit_path(root, "HitTestBehavior::PassThrough", out),
            Self::Drag => emit_path(root, "HitTestBehavior::Drag", out),
            Self::Resize(edge) => {
                emit_path(root, "HitTestBehavior::Resize(", out);
                edge.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for AnchorSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "AnchorSpec{rect: ", out);
        self.rect.emit_rust(root, out);
        out.push_str(",placement: ");
        self.placement.emit_rust(root, out);
        out.push_str(",alignment: ");
        self.alignment.emit_rust(root, out);
        out.push_str(",gap: ");
        self.gap.emit_rust(root, out);
        out.push_str(",room: ");
        self.room.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for LayoutCost {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Finite(v) => {
                emit_path(root, "LayoutCost::Finite(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Forbidden => emit_path(root, "LayoutCost::Forbidden", out),
        }
    }
}

impl EmitRust for ElasticFloor {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Px(v) => {
                emit_path(root, "ElasticFloor::Px(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Fraction(v) => {
                emit_path(root, "ElasticFloor::Fraction(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ElasticLength {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ElasticLength{floor: ", out);
        self.floor.emit_rust(root, out);
        out.push_str(",cost: ");
        self.cost.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for AxisElasticity {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "AxisElasticity{padding: ", out);
        self.padding.emit_rust(root, out);
        out.push_str(",gap: ");
        self.gap.emit_rust(root, out);
        out.push_str(",content_gap: ");
        self.content_gap.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for DiscreteCandidate {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "DiscreteCandidate{saves_px: ", out);
        self.saves_px.emit_rust(root, out);
        out.push_str(",cost: ");
        self.cost.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for DiscreteAdaptation {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "DiscreteAdaptation{candidates: ", out);
        self.candidates.emit_rust(root, out);
        out.push_str(",hysteresis_px: ");
        self.hysteresis_px.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for AdaptationProfile {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "AdaptationProfile{inline: ", out);
        self.inline.emit_rust(root, out);
        out.push_str(",block: ");
        self.block.emit_rust(root, out);
        out.push_str(",aggregate_children: ");
        self.aggregate_children.emit_rust(root, out);
        out.push_str(",solve_overflow: ");
        self.solve_overflow.emit_rust(root, out);
        out.push_str(",break_before: ");
        self.break_before.emit_rust(root, out);
        out.push_str(",break_after: ");
        self.break_after.emit_rust(root, out);
        out.push_str(",keep_with: ");
        self.keep_with.emit_rust(root, out);
        out.push_str(",discrete: ");
        self.discrete.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for IconPathCommand {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::MoveTo(p) => {
                emit_path(root, "IconPathCommand::MoveTo(", out);
                p.emit_rust(root, out);
                out.push(')');
            }
            Self::LineTo(p) => {
                emit_path(root, "IconPathCommand::LineTo(", out);
                p.emit_rust(root, out);
                out.push(')');
            }
            Self::CubicTo {
                control_a,
                control_b,
                to,
            } => {
                emit_path(root, "IconPathCommand::CubicTo{control_a: ", out);
                control_a.emit_rust(root, out);
                out.push_str(",control_b: ");
                control_b.emit_rust(root, out);
                out.push_str(",to: ");
                to.emit_rust(root, out);
                out.push('}');
            }
            Self::Close => emit_path(root, "IconPathCommand::Close", out),
        }
    }
}

impl EmitRust for IconShape {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Path(cmds) => {
                emit_path(root, "IconShape::Path(&[", out);
                emit_slice_items(cmds, root, out);
                out.push_str("])");
            }
            Self::Fill(cmds) => {
                emit_path(root, "IconShape::Fill(&[", out);
                emit_slice_items(cmds, root, out);
                out.push_str("])");
            }
            Self::Circle { center, radius } => {
                emit_path(root, "IconShape::Circle{center: ", out);
                center.emit_rust(root, out);
                out.push_str(",radius: ");
                radius.emit_rust(root, out);
                out.push('}');
            }
            Self::Rect {
                origin,
                size,
                filled,
            } => {
                emit_path(root, "IconShape::Rect{origin: ", out);
                origin.emit_rust(root, out);
                out.push_str(",size: ");
                size.emit_rust(root, out);
                out.push_str(",filled: ");
                filled.emit_rust(root, out);
                out.push('}');
            }
            Self::RoundedRect {
                origin,
                size,
                radius,
            } => {
                emit_path(root, "IconShape::RoundedRect{origin: ", out);
                origin.emit_rust(root, out);
                out.push_str(",size: ");
                size.emit_rust(root, out);
                out.push_str(",radius: ");
                radius.emit_rust(root, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for CustomCursorVariant {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "CustomCursorVariant{scale_milli: ", out);
        self.scale_milli.emit_rust(root, out);
        out.push_str(",width: ");
        self.width.emit_rust(root, out);
        out.push_str(",height: ");
        self.height.emit_rust(root, out);
        out.push_str(",hotspot: ");
        self.hotspot.emit_rust(root, out);
        out.push_str(",rgba: ::std::sync::Arc::from([");
        emit_slice_items(&self.rgba, root, out);
        out.push_str("])}");
    }
}

impl EmitRust for CustomCursorVector {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Shapes { shapes, color } => {
                emit_path(
                    root,
                    "CustomCursorVector::Shapes{shapes: ::std::sync::Arc::from([",
                    out,
                );
                emit_slice_items(shapes, root, out);
                out.push_str("]),color: ");
                color.emit_rust(root, out);
                out.push('}');
            }
            Self::Svg(bytes) => {
                emit_path(
                    root,
                    "CustomCursorVector::Svg(::std::sync::Arc::from([",
                    out,
                );
                emit_slice_items(bytes, root, out);
                out.push_str("]))");
            }
        }
    }
}

impl EmitRust for CustomCursorSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Raster { variants, fallback } => {
                emit_path(
                    root,
                    "CustomCursorSpec::Raster{variants: ::std::sync::Arc::from([",
                    out,
                );
                emit_slice_items(variants, root, out);
                out.push_str("]),fallback: ");
                fallback.emit_rust(root, out);
                out.push('}');
            }
            Self::Vector {
                source,
                logical_size,
                hotspot,
                fallback,
            } => {
                emit_path(root, "CustomCursorSpec::Vector{source: ", out);
                source.emit_rust(root, out);
                out.push_str(",logical_size: ");
                logical_size.emit_rust(root, out);
                out.push_str(",hotspot: ");
                hotspot.emit_rust(root, out);
                out.push_str(",fallback: ");
                fallback.emit_rust(root, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for CursorSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Default => emit_path(root, "CursorSpec::Default", out),
            Self::Pointer => emit_path(root, "CursorSpec::Pointer", out),
            Self::Text => emit_path(root, "CursorSpec::Text", out),
            Self::Move => emit_path(root, "CursorSpec::Move", out),
            Self::Grab => emit_path(root, "CursorSpec::Grab", out),
            Self::Grabbing => emit_path(root, "CursorSpec::Grabbing", out),
            Self::NotAllowed => emit_path(root, "CursorSpec::NotAllowed", out),
            Self::Crosshair => emit_path(root, "CursorSpec::Crosshair", out),
            Self::EwResize => emit_path(root, "CursorSpec::EwResize", out),
            Self::NsResize => emit_path(root, "CursorSpec::NsResize", out),
            Self::Help => emit_path(root, "CursorSpec::Help", out),
            Self::Wait => emit_path(root, "CursorSpec::Wait", out),
            Self::Progress => emit_path(root, "CursorSpec::Progress", out),
            Self::ZoomIn => emit_path(root, "CursorSpec::ZoomIn", out),
            Self::ZoomOut => emit_path(root, "CursorSpec::ZoomOut", out),
            Self::None => emit_path(root, "CursorSpec::None", out),
            Self::Custom(custom) => {
                emit_path(root, "CursorSpec::Custom(", out);
                custom.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

// ---- ThemeExpr & ThemeToken ----

impl EmitRust for ThemeToken {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Space(v) => {
                emit_path(root, "ThemeToken::Space(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Type(v) => {
                emit_path(root, "ThemeToken::Type(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Weight(v) => {
                emit_path(root, "ThemeToken::Weight(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Line(v) => {
                emit_path(root, "ThemeToken::Line(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Border(v) => {
                emit_path(root, "ThemeToken::Border(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Radius(v) => {
                emit_path(root, "ThemeToken::Radius(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Control(v) => {
                emit_path(root, "ThemeToken::Control(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::ControlPadding(v) => {
                emit_path(root, "ThemeToken::ControlPadding(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::PanelPaddingX => emit_path(root, "ThemeToken::PanelPaddingX", out),
            Self::MenuSurfacePadding => emit_path(root, "ThemeToken::MenuSurfacePadding", out),
            Self::MenuItemGap => emit_path(root, "ThemeToken::MenuItemGap", out),
            other => unreachable!("unsupported ThemeToken variant in codegen: {other:?}"),
        }
    }
}

impl EmitRust for ThemeExpr {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ThemeExpr{constant: ", out);
        self.constant.emit_rust(root, out);
        out.push_str(",terms: ");
        self.terms.emit_rust(root, out);
        out.push('}');
    }
}

// ---- PaintColor ----

impl EmitRust for SemanticColorMix {
    fn emit_rust(&self, root: &str, out: &mut String) {
        let (first, second, weight) = self.parts();
        emit_path(root, "SemanticColorMix::from_parts(", out);
        first.emit_rust(root, out);
        out.push(',');
        second.emit_rust(root, out);
        out.push(',');
        weight.emit_rust(root, out);
        out.push(')');
    }
}

impl EmitRust for ThemeColorPairRef {
    fn emit_rust(&self, root: &str, out: &mut String) {
        let (light, dark) = self.pair();
        emit_path(root, "intern_theme_color_pair(", out);
        light.emit_rust(root, out);
        out.push(',');
        dark.emit_rust(root, out);
        out.push(')');
    }
}

impl EmitRust for PaintColor {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Srgb { rgba } => {
                emit_path(root, "PaintColor::Srgb{rgba: ", out);
                rgba.emit_rust(root, out);
                out.push('}');
            }
            Self::Oklch { l, c, h, alpha } => {
                emit_path(root, "PaintColor::Oklch{l: ", out);
                l.emit_rust(root, out);
                out.push_str(",c: ");
                c.emit_rust(root, out);
                out.push_str(",h: ");
                h.emit_rust(root, out);
                out.push_str(",alpha: ");
                alpha.emit_rust(root, out);
                out.push('}');
            }
            Self::Hsv { h, s, v, alpha } => {
                emit_path(root, "PaintColor::Hsv{h: ", out);
                h.emit_rust(root, out);
                out.push_str(",s: ");
                s.emit_rust(root, out);
                out.push_str(",v: ");
                v.emit_rust(root, out);
                out.push_str(",alpha: ");
                alpha.emit_rust(root, out);
                out.push('}');
            }
            Self::LinearScRgb { channels, alpha } => {
                emit_path(root, "PaintColor::LinearScRgb{channels: ", out);
                channels.emit_rust(root, out);
                out.push_str(",alpha: ");
                alpha.emit_rust(root, out);
                out.push('}');
            }
            Self::ThemeRole { role } => {
                emit_path(root, "PaintColor::ThemeRole{role: ", out);
                role.emit_rust(root, out);
                out.push('}');
            }
            Self::ThemeMix(mix) => {
                emit_path(root, "PaintColor::ThemeMix(", out);
                mix.emit_rust(root, out);
                out.push(')');
            }
            Self::ThemePair(pair) => {
                emit_path(root, "PaintColor::ThemePair(", out);
                pair.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

// ---- Transforms ----

impl EmitRust for PaintTransform {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "PaintTransform{a: ", out);
        self.a.emit_rust(root, out);
        out.push_str(",b: ");
        self.b.emit_rust(root, out);
        out.push_str(",c: ");
        self.c.emit_rust(root, out);
        out.push_str(",d: ");
        self.d.emit_rust(root, out);
        out.push_str(",e: ");
        self.e.emit_rust(root, out);
        out.push_str(",f: ");
        self.f.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for PaintMat4 {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "PaintMat4{m: ", out);
        self.m.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for TransformOrigin {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "TransformOrigin{x: ", out);
        self.x.emit_rust(root, out);
        out.push_str(",y: ");
        self.y.emit_rust(root, out);
        out.push('}');
    }
}

// ---- Typography ----

impl EmitRust for TextDecorationLine {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "TextDecorationLine{underline: ", out);
        self.underline.emit_rust(root, out);
        out.push_str(",line_through: ");
        self.line_through.emit_rust(root, out);
        out.push_str(",overline: ");
        self.overline.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for TextEmphasisStyle {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::None => emit_path(root, "TextEmphasisStyle::None", out),
            Self::Mark { shape, open } => {
                emit_path(root, "TextEmphasisStyle::Mark{shape: ", out);
                shape.emit_rust(root, out);
                out.push_str(",open: ");
                open.emit_rust(root, out);
                out.push('}');
            }
            Self::String(s) => {
                emit_path(root, "TextEmphasisStyle::String(", out);
                s.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for TextEmphasisPosition {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "TextEmphasisPosition{under: ", out);
        self.under.emit_rust(root, out);
        out.push_str(",left: ");
        self.left.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for TextEmphasisColor {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::CurrentColor => emit_path(root, "TextEmphasisColor::CurrentColor", out),
            Self::Color(c) => {
                emit_path(root, "TextEmphasisColor::Color(", out);
                c.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for FontFeatureSetting {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "FontFeatureSetting{tag: ", out);
        self.tag.emit_rust(root, out);
        out.push_str(",value: ");
        self.value.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for FontVariationSetting {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "FontVariationSetting{tag: ", out);
        self.tag.emit_rust(root, out);
        out.push_str(",value: ");
        self.value.emit_rust(root, out);
        out.push('}');
    }
}

// ---- Grid ----

impl EmitRust for GridTrack {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Px(v) => {
                emit_path(root, "GridTrack::Px(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Percent(v) => {
                emit_path(root, "GridTrack::Percent(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Fr(v) => {
                emit_path(root, "GridTrack::Fr(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::MinMax { min_px, fr, max_px } => {
                emit_path(root, "GridTrack::MinMax{min_px: ", out);
                min_px.emit_rust(root, out);
                out.push_str(",fr: ");
                fr.emit_rust(root, out);
                out.push_str(",max_px: ");
                max_px.emit_rust(root, out);
                out.push('}');
            }
            Self::Auto => emit_path(root, "GridTrack::Auto", out),
            Self::FitContent { limit_px } => {
                emit_path(root, "GridTrack::FitContent{limit_px: ", out);
                limit_px.emit_rust(root, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for GridRepeatAuto {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "GridRepeatAuto{kind: ", out);
        self.kind.emit_rust(root, out);
        out.push_str(",tracks: ");
        self.tracks.emit_rust(root, out);
        out.push_str(",prefix: ");
        self.prefix.emit_rust(root, out);
        out.push_str(",suffix: ");
        self.suffix.emit_rust(root, out);
        out.push_str(",prefix_line_names: ");
        self.prefix_line_names.emit_rust(root, out);
        out.push_str(",pattern_line_names: ");
        self.pattern_line_names.emit_rust(root, out);
        out.push_str(",suffix_line_names: ");
        self.suffix_line_names.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for GridTemplateAreas {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "GridTemplateAreas{cells: ", out);
        self.cells.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for GridLine {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Auto => emit_path(root, "GridLine::Auto", out),
            Self::Index(i) => {
                emit_path(root, "GridLine::Index(", out);
                i.emit_rust(root, out);
                out.push(')');
            }
            Self::Span(n) => {
                emit_path(root, "GridLine::Span(", out);
                n.emit_rust(root, out);
                out.push(')');
            }
            Self::Name(name) => {
                emit_path(root, "GridLine::Name(", out);
                name.emit_rust(root, out);
                out.push(')');
            }
            Self::NthName(name, n) => {
                emit_path(root, "GridLine::NthName(", out);
                name.emit_rust(root, out);
                out.push(',');
                n.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

// ---- Paint / Materials / Effects ----

impl EmitRust for MaterialTexture {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "MaterialTexture{size: ", out);
        self.size.emit_rust(root, out);
        out.push_str(",rgba: ::std::sync::Arc::from([");
        emit_slice_items(&self.rgba, root, out);
        out.push_str("])}");
    }
}

impl EmitRust for MaterialRef {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "MaterialRef::new(", out);
        emit_path(root, "MaterialProgram::new(", out);
        let src: &str = &self.program().source;
        let _ = write!(out, "{src:?}))");
        let params = self.parameters();
        if params != [[0.0f32; 4]] {
            out.push_str(".with_parameters(::std::sync::Arc::from([");
            emit_slice_items(params, root, out);
            out.push_str("]))");
        }
        let textures = self.textures();
        if !textures.is_empty() {
            out.push_str(".with_textures(::std::sync::Arc::from([");
            emit_slice_items(textures, root, out);
            out.push_str("]))");
        }
    }
}

impl EmitRust for BoxShadowSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "BoxShadowSpec{offset_x: ", out);
        self.offset_x.emit_rust(root, out);
        out.push_str(",offset_y: ");
        self.offset_y.emit_rust(root, out);
        out.push_str(",blur_radius: ");
        self.blur_radius.emit_rust(root, out);
        out.push_str(",spread_radius: ");
        self.spread_radius.emit_rust(root, out);
        out.push_str(",color: ");
        self.color.emit_rust(root, out);
        out.push_str(",inset: ");
        self.inset.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ScrollbarSkin {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ScrollbarSkin{thickness: ", out);
        self.thickness.emit_rust(root, out);
        out.push_str(",thumb_thickness: ");
        self.thumb_thickness.emit_rust(root, out);
        out.push_str(",thumb_active_thickness: ");
        self.thumb_active_thickness.emit_rust(root, out);
        out.push_str(",track_color: ");
        self.track_color.emit_rust(root, out);
        out.push_str(",track_hot_color: ");
        self.track_hot_color.emit_rust(root, out);
        out.push_str(",thumb_color: ");
        self.thumb_color.emit_rust(root, out);
        out.push_str(",thumb_hot_color: ");
        self.thumb_hot_color.emit_rust(root, out);
        out.push_str(",track_border_color: ");
        self.track_border_color.emit_rust(root, out);
        out.push_str(",thumb_border_color: ");
        self.thumb_border_color.emit_rust(root, out);
        out.push_str(",over_media: ");
        self.over_media.emit_rust(root, out);
        out.push_str(",room: ");
        self.room.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ScrollEdgeFade {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ScrollEdgeFade{top: ", out);
        self.top.emit_rust(root, out);
        out.push_str(",bottom: ");
        self.bottom.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for TextShadowSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "TextShadowSpec{offset_x: ", out);
        self.offset_x.emit_rust(root, out);
        out.push_str(",offset_y: ");
        self.offset_y.emit_rust(root, out);
        out.push_str(",blur_radius: ");
        self.blur_radius.emit_rust(root, out);
        out.push_str(",color: ");
        self.color.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for TextStrokeSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "TextStrokeSpec{width: ", out);
        self.width.emit_rust(root, out);
        out.push_str(",color: ");
        self.color.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for GradientStop {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "GradientStop{position: ", out);
        self.position.emit_rust(root, out);
        out.push_str(",color: ");
        self.color.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for LinearGradient {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "LinearGradient{angle_deg: ", out);
        self.angle_deg.emit_rust(root, out);
        out.push_str(",stops: ");
        self.stops.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for RadialGradient {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "RadialGradient{circle: ", out);
        self.circle.emit_rust(root, out);
        out.push_str(",center: ");
        self.center.emit_rust(root, out);
        out.push_str(",stops: ");
        self.stops.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for CssGradient {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Linear(g) => {
                emit_path(root, "CssGradient::Linear(", out);
                g.emit_rust(root, out);
                out.push(')');
            }
            Self::Radial(g) => {
                emit_path(root, "CssGradient::Radial(", out);
                g.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for BackgroundPosition {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "BackgroundPosition{x: ", out);
        self.x.emit_rust(root, out);
        out.push_str(",y: ");
        self.y.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for BackgroundImage {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Gradient(g) => {
                emit_path(root, "BackgroundImage::Gradient(", out);
                g.emit_rust(root, out);
                out.push(')');
            }
            Self::Url {
                url,
                fit,
                size_width,
                size_height,
                position,
                repeat,
                sampling,
            } => {
                emit_path(root, "BackgroundImage::Url{url: ", out);
                url.emit_rust(root, out);
                out.push_str(",fit: ");
                fit.emit_rust(root, out);
                out.push_str(",size_width: ");
                size_width.emit_rust(root, out);
                out.push_str(",size_height: ");
                size_height.emit_rust(root, out);
                out.push_str(",position: ");
                position.emit_rust(root, out);
                out.push_str(",repeat: ");
                repeat.emit_rust(root, out);
                out.push_str(",sampling: ");
                sampling.emit_rust(root, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for BorderImageSlice {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Number(v) => {
                emit_path(root, "BorderImageSlice::Number(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Percent(v) => {
                emit_path(root, "BorderImageSlice::Percent(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for BorderImageSpec {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "BorderImageSpec{source: ", out);
        self.source.emit_rust(root, out);
        out.push_str(",slice: ");
        self.slice.emit_rust(root, out);
        out.push_str(",fill: ");
        self.fill.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for MaskImage {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Gradient(g) => {
                emit_path(root, "MaskImage::Gradient(", out);
                g.emit_rust(root, out);
                out.push(')');
            }
            Self::Url(u) => {
                emit_path(root, "MaskImage::Url(", out);
                u.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ClipInset {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ClipInset{top: ", out);
        self.top.emit_rust(root, out);
        out.push_str(",right: ");
        self.right.emit_rust(root, out);
        out.push_str(",bottom: ");
        self.bottom.emit_rust(root, out);
        out.push_str(",left: ");
        self.left.emit_rust(root, out);
        out.push_str(",round: ");
        self.round.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ClipPoint {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ClipPoint{x: ", out);
        self.x.emit_rust(root, out);
        out.push_str(",y: ");
        self.y.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ClipShapeRadius {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::ClosestSide => emit_path(root, "ClipShapeRadius::ClosestSide", out),
            Self::FarthestSide => emit_path(root, "ClipShapeRadius::FarthestSide", out),
            Self::Length(len) => {
                emit_path(root, "ClipShapeRadius::Length(", out);
                len.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ClipCircle {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ClipCircle{radius: ", out);
        self.radius.emit_rust(root, out);
        out.push_str(",cx: ");
        self.cx.emit_rust(root, out);
        out.push_str(",cy: ");
        self.cy.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ClipEllipse {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ClipEllipse{rx: ", out);
        self.rx.emit_rust(root, out);
        out.push_str(",ry: ");
        self.ry.emit_rust(root, out);
        out.push_str(",cx: ");
        self.cx.emit_rust(root, out);
        out.push_str(",cy: ");
        self.cy.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ClipPath {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Inset(v) => {
                emit_path(root, "ClipPath::Inset(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Polygon(pts) => {
                emit_path(root, "ClipPath::Polygon(", out);
                pts.emit_rust(root, out);
                out.push(')');
            }
            Self::Circle(v) => {
                emit_path(root, "ClipPath::Circle(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Ellipse(v) => {
                emit_path(root, "ClipPath::Ellipse(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for FilterDropShadow {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "FilterDropShadow{offset_x: ", out);
        self.offset_x.emit_rust(root, out);
        out.push_str(",offset_y: ");
        self.offset_y.emit_rust(root, out);
        out.push_str(",blur_radius: ");
        self.blur_radius.emit_rust(root, out);
        out.push_str(",color: ");
        self.color.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for ColorFilter {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "ColorFilter{brightness: ", out);
        self.brightness.emit_rust(root, out);
        out.push_str(",saturate: ");
        self.saturate.emit_rust(root, out);
        out.push_str(",contrast: ");
        self.contrast.emit_rust(root, out);
        out.push_str(",hue_rotate_deg: ");
        self.hue_rotate_deg.emit_rust(root, out);
        out.push_str(",invert: ");
        self.invert.emit_rust(root, out);
        out.push_str(",opacity: ");
        self.opacity.emit_rust(root, out);
        out.push_str(",blur_radius: ");
        self.blur_radius.emit_rust(root, out);
        out.push_str(",drop_shadow: ");
        self.drop_shadow.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for BackdropFilter {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "BackdropFilter{blur_radius: ", out);
        self.blur_radius.emit_rust(root, out);
        out.push_str(",saturate: ");
        self.saturate.emit_rust(root, out);
        out.push('}');
    }
}

// ---- MediaQuery & FontFaceRule ----

impl EmitRust for MediaFeature {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Width(cmp, px) => {
                emit_path(root, "MediaFeature::Width(", out);
                cmp.emit_rust(root, out);
                out.push(',');
                px.emit_rust(root, out);
                out.push(')');
            }
            Self::Height(cmp, px) => {
                emit_path(root, "MediaFeature::Height(", out);
                cmp.emit_rust(root, out);
                out.push(',');
                px.emit_rust(root, out);
                out.push(')');
            }
            Self::AspectRatio(cmp, n, d) => {
                emit_path(root, "MediaFeature::AspectRatio(", out);
                cmp.emit_rust(root, out);
                out.push(',');
                n.emit_rust(root, out);
                out.push(',');
                d.emit_rust(root, out);
                out.push(')');
            }
            Self::Orientation(v) => {
                emit_path(root, "MediaFeature::Orientation(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::PrefersColorScheme(v) => {
                emit_path(root, "MediaFeature::PrefersColorScheme(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::PrefersReducedMotion(v) => {
                emit_path(root, "MediaFeature::PrefersReducedMotion(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::PrefersContrast(v) => {
                emit_path(root, "MediaFeature::PrefersContrast(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Hover(v) => {
                emit_path(root, "MediaFeature::Hover(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Pointer(v) => {
                emit_path(root, "MediaFeature::Pointer(", out);
                v.emit_rust(root, out);
                out.push(')');
            }
            Self::Resolution(cmp, dppx) => {
                emit_path(root, "MediaFeature::Resolution(", out);
                cmp.emit_rust(root, out);
                out.push(',');
                dppx.emit_rust(root, out);
                out.push(')');
            }
            Self::Unknown => emit_path(root, "MediaFeature::Unknown", out),
        }
    }
}

impl EmitRust for MediaCondition {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Feature(f) => {
                emit_path(root, "MediaCondition::Feature(", out);
                f.emit_rust(root, out);
                out.push(')');
            }
            Self::Not(inner) => {
                emit_path(root, "MediaCondition::Not(::std::boxed::Box::new(", out);
                inner.emit_rust(root, out);
                out.push_str("))");
            }
            Self::And(items) => {
                emit_path(root, "MediaCondition::And(", out);
                items.emit_rust(root, out);
                out.push(')');
            }
            Self::Or(items) => {
                emit_path(root, "MediaCondition::Or(", out);
                items.emit_rust(root, out);
                out.push(')');
            }
            Self::Unknown => emit_path(root, "MediaCondition::Unknown", out),
        }
    }
}

impl EmitRust for MediaQuery {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "MediaQuery{negated: ", out);
        self.negated.emit_rust(root, out);
        out.push_str(",media_type: ");
        self.media_type.emit_rust(root, out);
        out.push_str(",condition: ");
        self.condition.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for MediaQueryList {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "MediaQueryList{queries: ", out);
        self.queries.emit_rust(root, out);
        out.push('}');
    }
}

impl EmitRust for FontFaceSource {
    fn emit_rust(&self, root: &str, out: &mut String) {
        match self {
            Self::Url { href, format } => {
                emit_path(root, "FontFaceSource::Url{href: ", out);
                href.emit_rust(root, out);
                out.push_str(",format: ");
                format.emit_rust(root, out);
                out.push('}');
            }
            Self::Local(name) => {
                emit_path(root, "FontFaceSource::Local(", out);
                name.emit_rust(root, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for FontFaceRule {
    fn emit_rust(&self, root: &str, out: &mut String) {
        emit_path(root, "FontFaceRule{family: ", out);
        self.family.emit_rust(root, out);
        out.push_str(",sources: ");
        self.sources.emit_rust(root, out);
        out.push_str(",weight: ");
        self.weight.emit_rust(root, out);
        out.push_str(",weight_end: ");
        self.weight_end.emit_rust(root, out);
        out.push_str(",style: ");
        self.style.emit_rust(root, out);
        out.push_str(",base_href: ");
        self.base_href.emit_rust(root, out);
        out.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delimiters_balanced(code: &str) -> bool {
        let mut stack = Vec::new();
        let mut in_string = false;
        let mut escape = false;
        for ch in code.chars() {
            if in_string {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '(' | '[' | '{' => stack.push(ch),
                ')' | ']' | '}' => {
                    let expected = match ch {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    if stack.pop() != Some(expected) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        !in_string && stack.is_empty()
    }

    #[test]
    fn emits_all_242_sample_style_ops_without_panicking() {
        let samples = nana_ui_core::sample_style_ops();
        assert_eq!(samples.len(), nana_ui_core::STYLE_OP_COUNT);
        let root = "::nana_ui_runtime::view::__patch";
        for op in &samples {
            let mut stmt = String::new();
            emit_style_op_rust(op, root, &mut stmt);
            assert!(
                !stmt.is_empty(),
                "empty emission for setter {}",
                op.setter_path()
            );
            let expected_prefix = format!("{root}::set::{}(__nana_layout,", op.setter_path());
            assert!(
                stmt.starts_with(&expected_prefix),
                "unexpected prefix for {}: {stmt}",
                op.setter_path()
            );
            assert!(
                stmt.ends_with(");"),
                "missing trailing `);` for {}: {stmt}",
                op.setter_path()
            );
            assert!(
                !stmt.contains(":::"),
                "invalid `:::` token in emitted Rust for {}: {stmt}",
                op.setter_path()
            );
            assert!(
                delimiters_balanced(&stmt),
                "unbalanced delimiters for {}: {stmt}",
                op.setter_path()
            );
        }

        let combined = emit_style_ops_rust(&samples, root);
        assert!(delimiters_balanced(&combined));
    }

    #[test]
    fn emits_media_query_list_and_font_face_rule() {
        let list = MediaQueryList {
            queries: vec![
                MediaQuery::with_type(
                    false,
                    MediaType::Screen,
                    Some(MediaCondition::And(vec![
                        MediaCondition::Feature(MediaFeature::min_width(768.0)),
                        MediaCondition::Not(Box::new(MediaCondition::Feature(
                            MediaFeature::PrefersColorScheme(MediaColorScheme::Dark),
                        ))),
                    ])),
                ),
                MediaQuery::from_condition(MediaCondition::Or(vec![
                    MediaCondition::Feature(MediaFeature::Orientation(MediaOrientation::Portrait)),
                    MediaCondition::Unknown,
                ])),
            ],
        };
        let mq_code = emit_media_query_list_rust(&list, "::nana_ui_core");
        assert!(mq_code.starts_with("::nana_ui_core::MediaQueryList{"));
        assert!(!mq_code.contains(":::"));
        assert!(delimiters_balanced(&mq_code));

        let face = FontFaceRule {
            family: "Inter".into(),
            sources: vec![
                FontFaceSource::local("Inter"),
                FontFaceSource::url_with_format("nana://res/inter.woff2", "woff2"),
            ],
            weight: Some(300),
            weight_end: Some(700),
            style: Some(FontStyleSpec::Italic),
            base_href: Some("nana://res/app.css".into()),
        };
        let ff_code = emit_font_face_rule_rust(&face, "::nana_ui_core");
        assert!(ff_code.starts_with("::nana_ui_core::FontFaceRule{"));
        assert!(!ff_code.contains(":::"));
        assert!(delimiters_balanced(&ff_code));
    }
}
