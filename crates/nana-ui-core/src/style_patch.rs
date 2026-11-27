//! Compiled style patches, typed [`StyleOp`] leaves, and direct field setters
//! without runtime JSON.
//!
//! One declarative table ([`define_style_ops!`]) is the single source of truth
//! for both [`StyleOp`] and [`set`]: every leaf path named by
//! [`crate::LayoutStyle::overlay_leaf_paths`] has a matching [`StyleOp`]
//! variant and a matching [`set`] function whose module path mirrors the dotted
//! field path (`theme.gap` -> `set::theme::gap`).

/// A compiled rule's field assignments.
#[derive(Clone, Copy)]
pub struct StylePatch {
    apply: fn(&mut crate::LayoutStyle),
}

impl StylePatch {
    pub const fn new(apply: fn(&mut crate::LayoutStyle)) -> Self {
        Self { apply }
    }

    #[inline]
    pub fn apply(&self, layout: &mut crate::LayoutStyle) {
        (self.apply)(layout);
    }
}

/// Dev-only data patch: typed [`StyleOp`] leaf writes (preferred), with legacy
/// `(layout, written)` retained transiently during Wave 1 migration.
/// Never constructed in product builds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HotStylePatch {
    pub ops: Vec<StyleOp>,
    pub layout: crate::LayoutStyle,
    pub written: Vec<String>,
}

impl HotStylePatch {
    /// Construct a data patch from typed [`StyleOp`] leaf operations.
    pub fn from_ops(ops: Vec<StyleOp>) -> Self {
        Self {
            ops,
            layout: crate::LayoutStyle::default(),
            written: Vec::new(),
        }
    }

    /// Legacy constructor taking a compiled `LayoutStyle` plus dotted leaf paths.
    pub fn new(layout: crate::LayoutStyle, written: Vec<String>) -> Self {
        Self {
            ops: Vec::new(),
            layout,
            written,
        }
    }

    pub fn apply(&self, target: &mut crate::LayoutStyle) {
        if !self.ops.is_empty() {
            for op in &self.ops {
                op.apply(target);
            }
        } else {
            target.apply_written_leaves(&self.layout, &self.written);
        }
    }
}

/// Container-query axis carried by [`HotSheetQuery`] without depending on
/// runtime responsive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotSheetAxis {
    Inline,
    Block,
    Width,
    Height,
}

/// Container-query guard on a [`HotStyleRule`].
#[derive(Debug, Clone, PartialEq)]
pub struct HotSheetQuery {
    pub name: Option<String>,
    pub axis: HotSheetAxis,
    pub intervals: Vec<(f32, f32)>,
}

impl HotSheetQuery {
    pub fn holds_at(&self, extent: f32) -> bool {
        self.intervals
            .iter()
            .any(|&(lo, hi)| lo <= extent && extent < hi)
    }
}

/// One dev-only hot-reloadable stylesheet rule in cascade order.
#[derive(Debug, Clone, PartialEq)]
pub struct HotStyleRule {
    pub classes: Vec<String>,
    pub pseudo: u8,
    pub patch: HotStylePatch,
    pub query: Option<HotSheetQuery>,
}

/// A view's `<style>` rules as pure data for dev-only `<style>` hot reload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HotStyleSheet {
    pub rules: Vec<HotStyleRule>,
}

macro_rules! define_style_ops {
    (
        top {
            $($top_var:ident => $top_field:ident : $top_ty:ty = $top_sample:expr),* $(,)?
        }
        $(
            group $group:ident {
                fields {
                    $($g_var:ident => $g_field:ident : $g_ty:ty = $g_sample:expr),* $(,)?
                }
                nested {
                    $(
                        $sub:ident {
                            $($s_var:ident => $s_field:ident : $s_ty:ty = $s_sample:expr),* $(,)?
                        }
                    )*
                }
            }
        )*
    ) => {
        /// Exhaustive typed leaf-write operation on [`crate::LayoutStyle`].
        ///
        /// Generated together with [`set`] from one declarative table so the
        /// typed IR and direct compiled setters can never drift.
        #[derive(Debug, Clone, PartialEq)]
        pub enum StyleOp {
            $(
                $top_var($top_ty),
            )*
            $(
                $(
                    $g_var($g_ty),
                )*
                $(
                    $(
                        $s_var($s_ty),
                    )*
                )*
            )*
        }

        /// Total number of writable leaf fields on [`crate::LayoutStyle`]
        /// covered by [`StyleOp`] and [`set`].
        pub const STYLE_OP_COUNT: usize = 0
            $(+ { let _ = stringify!($top_var); 1 })*
            $(
                $(+ { let _ = stringify!($g_var); 1 })*
                $(
                    $(+ { let _ = stringify!($s_var); 1 })*
                )*
            )*;

        const _: () = assert!(
            STYLE_OP_COUNT == crate::LayoutStyle::OVERLAY_LEAVES,
            "StyleOp table must cover every LayoutStyle overlay leaf exhaustively"
        );

        impl StyleOp {
            /// Apply this leaf assignment onto `layout`, preserving `Shared<T>`
            /// copy-on-write so setting an unchanged/default value never
            /// materializes a shared default `Arc`.
            pub fn apply(&self, layout: &mut crate::LayoutStyle) {
                match self {
                    $(
                        Self::$top_var(value) => {
                            layout.$top_field = value.clone();
                        }
                    )*
                    $(
                        $(
                            Self::$g_var(value) => {
                                if layout.$group.$g_field != *value {
                                    layout.$group.$g_field = value.clone();
                                }
                            }
                        )*
                        $(
                            $(
                                Self::$s_var(value) => {
                                    if layout.$group.$sub.$s_field != *value {
                                        layout.$group.$sub.$s_field = value.clone();
                                    }
                                }
                            )*
                        )*
                    )*
                }
            }

            /// Move-apply this leaf assignment onto `layout` without cloning,
            /// preserving `Shared<T>` copy-on-write.
            pub fn apply_into(self, layout: &mut crate::LayoutStyle) {
                match self {
                    $(
                        Self::$top_var(value) => set::$top_field(layout, value),
                    )*
                    $(
                        $(
                            Self::$g_var(value) => set::$group::$g_field(layout, value),
                        )*
                        $(
                            $(
                                Self::$s_var(value) => set::$group::$sub::$s_field(layout, value),
                            )*
                        )*
                    )*
                }
            }

            /// Dotted leaf path matching [`crate::LayoutStyle::overlay_leaf_paths`]
            /// (e.g. `"padding_left"`, `"paint.elevation"`, `"paint.outline.width"`).
            pub const fn field_path(&self) -> &'static str {
                match self {
                    $(
                        Self::$top_var(_) => stringify!($top_field),
                    )*
                    $(
                        $(
                            Self::$g_var(_) => concat!(stringify!($group), ".", stringify!($g_field)),
                        )*
                        $(
                            $(
                                Self::$s_var(_) => concat!(
                                    stringify!($group),
                                    ".",
                                    stringify!($sub),
                                    ".",
                                    stringify!($s_field)
                                ),
                            )*
                        )*
                    )*
                }
            }

            /// Rust setter path relative to [`set`]
            /// (e.g. `"padding_left"`, `"paint::elevation"`, `"paint::outline::width"`).
            pub const fn setter_path(&self) -> &'static str {
                match self {
                    $(
                        Self::$top_var(_) => stringify!($top_field),
                    )*
                    $(
                        $(
                            Self::$g_var(_) => concat!(stringify!($group), "::", stringify!($g_field)),
                        )*
                        $(
                            $(
                                Self::$s_var(_) => concat!(
                                    stringify!($group),
                                    "::",
                                    stringify!($sub),
                                    "::",
                                    stringify!($s_field)
                                ),
                            )*
                        )*
                    )*
                }
            }

            /// Extract a [`StyleOp`] from `layout` for a dotted leaf path named
            /// by [`crate::LayoutStyle::overlay_leaf_paths`].
            pub fn from_layout_path(layout: &crate::LayoutStyle, path: &str) -> Option<Self> {
                match path {
                    $(
                        stringify!($top_field) => Some(Self::$top_var(layout.$top_field.clone())),
                    )*
                    $(
                        $(
                            concat!(stringify!($group), ".", stringify!($g_field)) => {
                                Some(Self::$g_var(layout.$group.$g_field.clone()))
                            }
                        )*
                        $(
                            $(
                                concat!(
                                    stringify!($group),
                                    ".",
                                    stringify!($sub),
                                    ".",
                                    stringify!($s_field)
                                ) => Some(Self::$s_var(layout.$group.$sub.$s_field.clone())),
                            )*
                        )*
                    )*
                    _ => None,
                }
            }

            /// One populated sample [`StyleOp`] for every variant (count == [`STYLE_OP_COUNT`]).
            ///
            /// Downstream codegen tests iterate this list to compile-check every
            /// [`StyleOp`] variant and referenced core type against `view::__patch`.
            pub fn all_sample_variants() -> Vec<Self> {
                sample_style_ops()
            }
        }

        /// Returns one populated, non-default sample [`StyleOp`] for every leaf
        /// variant in [`StyleOp`] (`len() == STYLE_OP_COUNT`).
        pub fn sample_style_ops() -> Vec<StyleOp> {
            vec![
                $(
                    StyleOp::$top_var($top_sample),
                )*
                $(
                    $(
                        StyleOp::$g_var($g_sample),
                    )*
                    $(
                        $(
                            StyleOp::$s_var($s_sample),
                        )*
                    )*
                )*
            ]
        }

        /// Direct leaf setters on [`crate::LayoutStyle`] for compiled style patches.
        ///
        /// Generated from the exact same [`define_style_ops!`] table as [`StyleOp`].
        pub mod set {
            $(
                #[inline]
                pub fn $top_field(layout: &mut crate::LayoutStyle, value: $top_ty) {
                    layout.$top_field = value;
                }
            )*

            $(
                pub mod $group {
                    $(
                        #[inline]
                        pub fn $g_field(layout: &mut crate::LayoutStyle, value: $g_ty) {
                            if layout.$group.$g_field == value {
                                return;
                            }
                            layout.$group.$g_field = value;
                        }
                    )*

                    $(
                        pub mod $sub {
                            $(
                                #[inline]
                                pub fn $s_field(layout: &mut crate::LayoutStyle, value: $s_ty) {
                                    if layout.$group.$sub.$s_field == value {
                                        return;
                                    }
                                    layout.$group.$sub.$s_field = value;
                                }
                            )*
                        }
                    )*
                }
            )*
        }
    };
}

fn sample_theme_expr() -> crate::ThemeExpr {
    crate::ThemeExpr::token(crate::ThemeToken::Space(crate::SpacingStep::Md))
}

fn sample_paint_color() -> crate::PaintColor {
    crate::PaintColor::theme_role(crate::SemanticColorRole::Accent)
}

fn sample_material_ref() -> crate::MaterialRef {
    crate::MaterialRef::new(crate::MaterialProgram::new(
        "fn material(input: MaterialInput) -> vec4<f32> { return input.color; }",
    ))
}

define_style_ops! {
    top {
        LayoutIsolation => layout_isolation: bool = true,
        Direction => direction: Option<crate::FlexDirection> = Some(crate::FlexDirection::Row),
        Dir => dir: Option<crate::DirSpec> = Some(crate::DirSpec::Rtl),
        WritingMode => writing_mode: Option<crate::WritingModeSpec> = Some(crate::WritingModeSpec::VerticalRl),
        FlexReverse => flex_reverse: bool = true,
        Order => order: i32 = 2,
        FlexWrap => flex_wrap: crate::FlexWrap = crate::FlexWrap::Wrap,
        EqualWrap => equal_wrap: bool = true,
        Display => display: Option<crate::DisplaySpec> = Some(crate::DisplaySpec::Grid),
        BoxSizing => box_sizing: crate::BoxSizing = crate::BoxSizing::ContentBox,
        Position => position: crate::PositionSpec = crate::PositionSpec::Relative,
        Anchor => anchor: Option<std::sync::Arc<crate::AnchorSpec>> = Some(std::sync::Arc::new(crate::AnchorSpec {
            rect: [10.0, 20.0, 100.0, 32.0],
            placement: crate::PopoverPlacement::Bottom,
            alignment: crate::PopoverAlignment::Start,
            gap: 4.0,
            room: None,
        })),
        ZIndex => z_index: Option<i32> = Some(10),
        Isolation => isolation: bool = true,
        Gap => gap: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
        RowGap => row_gap: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(6.0)),
        ColumnGap => column_gap: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(12.0)),
        Padding => padding: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(12.0)),
        PaddingTop => padding_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
        PaddingRight => padding_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
        PaddingBottom => padding_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
        PaddingLeft => padding_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
        Margin => margin: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
        MarginTop => margin_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(2.0)),
        MarginRight => margin_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(6.0)),
        MarginBottom => margin_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(2.0)),
        MarginLeft => margin_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(6.0)),
        OffsetTop => offset_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(1.0)),
        OffsetRight => offset_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(2.0)),
        OffsetBottom => offset_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(3.0)),
        OffsetLeft => offset_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
        Width => width: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(120.0)),
        Height => height: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(40.0)),
        MinWidth => min_width: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(60.0)),
        MaxWidth => max_width: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(320.0)),
        MinHeight => min_height: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(24.0)),
        MaxHeight => max_height: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(240.0)),
        AllowShrink => allow_shrink: bool = true,
        AlignItems => align_items: crate::AlignSpec = crate::AlignSpec::Center,
        AlignSelf => align_self: Option<crate::AlignSpec> = Some(crate::AlignSpec::Stretch),
        AlignContent => align_content: crate::JustifySpec = crate::JustifySpec::Center,
        JustifyContent => justify_content: crate::JustifySpec = crate::JustifySpec::SpaceBetween,
        JustifyItems => justify_items: Option<crate::AlignSpec> = Some(crate::AlignSpec::Center),
        JustifySelf => justify_self: Option<crate::AlignSpec> = Some(crate::AlignSpec::End),
        FlexGrow => flex_grow: Option<f32> = Some(1.0),
        FlexShrink => flex_shrink: Option<f32> = Some(1.0),
        FlexBasis => flex_basis: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(100.0)),
        Adaptation => adaptation: Option<std::sync::Arc<crate::dynamic_layout::AdaptationProfile>> =
            Some(std::sync::Arc::new(crate::dynamic_layout::AdaptationProfile::default())),
        OverflowX => overflow_x: crate::OverflowSpec = crate::OverflowSpec::Hidden,
        OverflowY => overflow_y: crate::OverflowSpec = crate::OverflowSpec::Auto,
        TextOverflowEllipsis => text_overflow_ellipsis: bool = true,
        PointerEvents => pointer_events: Option<crate::PointerEventsSpec> = Some(crate::PointerEventsSpec::None),
        HitTest => hit_test: Option<crate::HitTestBehavior> = Some(crate::HitTestBehavior::Drag),
        Inert => inert: bool = true,
        Cursor => cursor: Option<crate::CursorSpec> = Some(crate::CursorSpec::Pointer),
        UserSelect => user_select: Option<crate::UserSelectSpec> = Some(crate::UserSelectSpec::Text),
        WhiteSpace => white_space: crate::WhiteSpaceSpec = crate::WhiteSpaceSpec::Nowrap,
        AspectRatio => aspect_ratio: Option<f32> = Some(16.0 / 9.0),
        TextAlign => text_align: crate::TextAlignSpec = crate::TextAlignSpec::Center,
        FontSize => font_size: Option<f32> = Some(14.0),
        FontWeight => font_weight: Option<u16> = Some(600),
        FontItalic => font_italic: Option<bool> = Some(true),
        LineHeight => line_height: Option<crate::LineHeightSpec> = Some(crate::LineHeightSpec::Relative(1.4)),
        Hidden => hidden: bool = true,
        Opacity => opacity: Option<f32> = Some(0.8),
        BorderRadius => border_radius: Option<f32> = Some(8.0),
        BorderWidth => border_width: Option<f32> = Some(1.0),
        BorderStyle => border_style: Option<crate::BorderStyle> = Some(crate::BorderStyle::Solid),
    }

    group theme {
        fields {
            ThemeGap => gap: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeRowGap => row_gap: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeColumnGap => column_gap: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemePadding => padding: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemePaddingTop => padding_top: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemePaddingRight => padding_right: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemePaddingBottom => padding_bottom: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemePaddingLeft => padding_left: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMargin => margin: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMarginTop => margin_top: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMarginRight => margin_right: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMarginBottom => margin_bottom: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMarginLeft => margin_left: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeOffsetTop => offset_top: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeOffsetRight => offset_right: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeOffsetBottom => offset_bottom: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeOffsetLeft => offset_left: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeWidth => width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeHeight => height: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMinWidth => min_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMaxWidth => max_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMinHeight => min_height: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeMaxHeight => max_height: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeFlexBasis => flex_basis: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeFontSize => font_size: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeFontWeight => font_weight: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeLineHeight => line_height: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeLetterSpacing => letter_spacing: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderWidth => border_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderTopWidth => border_top_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderRightWidth => border_right_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderBottomWidth => border_bottom_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderLeftWidth => border_left_width: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeBorderRadius => border_radius: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
            ThemeScrollbarRoom => scrollbar_room: Option<crate::ThemeExpr> = Some(sample_theme_expr()),
        }
        nested {}
    }

    group paint_colors {
        fields {
            Color => color: Option<crate::PaintColor> = Some(sample_paint_color()),
            Background => background: Option<crate::PaintColor> = Some(sample_paint_color()),
            BorderColor => border: Option<crate::PaintColor> = Some(sample_paint_color()),
            BorderTopColor => border_top: Option<crate::PaintColor> = Some(sample_paint_color()),
            BorderRightColor => border_right: Option<crate::PaintColor> = Some(sample_paint_color()),
            BorderBottomColor => border_bottom: Option<crate::PaintColor> = Some(sample_paint_color()),
            BorderLeftColor => border_left: Option<crate::PaintColor> = Some(sample_paint_color()),
            OutlineColor => outline: Option<crate::PaintColor> = Some(sample_paint_color()),
            SelectionBackground => selection_background: Option<crate::PaintColor> = Some(sample_paint_color()),
            SelectionColor => selection_color: Option<crate::PaintColor> = Some(sample_paint_color()),
        }
        nested {}
    }

    group transforms {
        fields {
            Transform => transform: Option<crate::PaintTransform> = Some(crate::PaintTransform::scaling(1.05, 1.05)),
            Transform3d => transform_3d: Option<crate::PaintMat4> = Some(crate::PaintMat4::translation(4.0, 0.0, 0.0)),
            UnsupportedTransform => unsupported_transform: Option<String> = Some("skewZ(10deg)".into()),
            TransformOrigin => origin: Option<crate::TransformOrigin> = Some(crate::TransformOrigin {
                x: crate::LengthSpec::Px(10.0),
                y: crate::LengthSpec::Px(20.0),
            }),
            CssPerspective => css_perspective: Option<f32> = Some(800.0),
            PerspectiveOrigin => perspective_origin: Option<crate::TransformOrigin> = Some(crate::TransformOrigin {
                x: crate::LengthSpec::Percent(25.0),
                y: crate::LengthSpec::Percent(75.0),
            }),
            Preserve3d => preserve_3d: bool = true,
            BackfaceVisibility => backface_visibility: crate::BackfaceVisibility = crate::BackfaceVisibility::Hidden,
            TransformBox => transform_box: crate::TransformBox = crate::TransformBox::ContentBox,
        }
        nested {}
    }

    group logical_edges {
        fields {}
        nested {
            padding {
                LogicalPaddingInlineStart => inline_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalPaddingInlineEnd => inline_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalPaddingBlockStart => block_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalPaddingBlockEnd => block_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalPaddingPhysTop => phys_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalPaddingPhysRight => phys_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalPaddingPhysBottom => phys_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalPaddingPhysLeft => phys_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalPaddingGenerations => generations: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8],
                LogicalPaddingNextGen => next_gen: u32 = 9,
                LogicalPaddingDerived => derived: u8 = 0b1111,
            }
            margin {
                LogicalMarginInlineStart => inline_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalMarginInlineEnd => inline_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalMarginBlockStart => block_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalMarginBlockEnd => block_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalMarginPhysTop => phys_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalMarginPhysRight => phys_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalMarginPhysBottom => phys_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalMarginPhysLeft => phys_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalMarginGenerations => generations: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8],
                LogicalMarginNextGen => next_gen: u32 = 9,
                LogicalMarginDerived => derived: u8 = 0b1111,
            }
            inset {
                LogicalInsetInlineStart => inline_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalInsetInlineEnd => inline_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalInsetBlockStart => block_start: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalInsetBlockEnd => block_end: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalInsetPhysTop => phys_top: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalInsetPhysRight => phys_right: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalInsetPhysBottom => phys_bottom: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(4.0)),
                LogicalInsetPhysLeft => phys_left: Option<crate::LengthSpec> = Some(crate::LengthSpec::Px(8.0)),
                LogicalInsetGenerations => generations: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8],
                LogicalInsetNextGen => next_gen: u32 = 9,
                LogicalInsetDerived => derived: u8 = 0b1111,
            }
        }
    }

    group text_ext {
        fields {
            FontFamily => font_family: Option<String> = Some("Inter".into()),
            LetterSpacing => letter_spacing: Option<f32> = Some(0.5),
            LineClamp => line_clamp: Option<u16> = Some(3),
            WordBreak => word_break: Option<crate::WordBreakSpec> = Some(crate::WordBreakSpec::BreakAll),
            OverflowWrap => overflow_wrap: Option<crate::OverflowWrapSpec> = Some(crate::OverflowWrapSpec::Anywhere),
            TextDecoration => text_decoration: Option<crate::TextDecorationLine> = Some(crate::TextDecorationLine {
                underline: true,
                line_through: false,
                overline: false,
            }),
            TextEmphasisStyle => text_emphasis_style: Option<crate::typography::TextEmphasisStyle> =
                Some(crate::typography::TextEmphasisStyle::Mark {
                    shape: Some(crate::typography::TextEmphasisShape::Dot),
                    open: false,
                }),
            TextEmphasisPosition => text_emphasis_position: Option<crate::typography::TextEmphasisPosition> =
                Some(crate::typography::TextEmphasisPosition::UNDER),
            TextEmphasisColor => text_emphasis_color: Option<crate::typography::TextEmphasisColor> =
                Some(crate::typography::TextEmphasisColor::Color(sample_paint_color())),
            FontFeatures => font_features: Option<Vec<crate::FontFeatureSetting>> = Some(vec![
                crate::FontFeatureSetting {
                    tag: *b"liga",
                    value: 1,
                },
            ]),
            FontVariationSettings => font_variation_settings: Option<Vec<crate::typography::FontVariationSetting>> = Some(vec![
                crate::typography::FontVariationSetting {
                    tag: *b"wght",
                    value: 550.0,
                },
            ]),
            FontKerning => font_kerning: Option<crate::typography::FontKerningSpec> =
                Some(crate::typography::FontKerningSpec::None),
            LineBreak => line_break: Option<crate::typography::LineBreakSpec> =
                Some(crate::typography::LineBreakSpec::Strict),
            Hyphens => hyphens: Option<crate::typography::HyphensSpec> =
                Some(crate::typography::HyphensSpec::Auto),
            UnsupportedFontVariation => unsupported_font_variation: bool = true,
            UnsupportedWritingMode => unsupported_writing_mode: bool = true,
            TextOrientation => text_orientation: Option<crate::TextOrientationSpec> =
                Some(crate::TextOrientationSpec::Upright),
            DominantBaseline => dominant_baseline: Option<crate::typography::DominantBaselineSpec> =
                Some(crate::typography::DominantBaselineSpec::Central),
            VerticalAlign => vertical_align: crate::VerticalAlignSpec = crate::VerticalAlignSpec::Middle,
            PlaceholderColor => placeholder_color: Option<[f32; 4]> = Some([0.5, 0.5, 0.5, 0.8]),
            PlaceholderOpacity => placeholder_opacity: Option<f32> = Some(0.6),
        }
        nested {
            text_typography {
                TextSpacingTrim => spacing_trim: Option<crate::typography::TextSpacingTrimSpec> =
                    Some(crate::typography::TextSpacingTrimSpec::Normal),
                TextAutospace => autospace: Option<crate::typography::TextAutospaceSpec> =
                    Some(crate::typography::TextAutospaceSpec::Normal),
                TextJustify => justify: Option<crate::typography::TextJustifySpec> =
                    Some(crate::typography::TextJustifySpec::InterCharacter),
                TextWrapStyle => wrap_style: Option<crate::typography::TextWrapStyleSpec> =
                    Some(crate::typography::TextWrapStyleSpec::Pretty),
                TextJustifyLines => justify_lines: Option<bool> = Some(true),
            }
        }
    }

    group grid {
        fields {
            GridColumns => columns: Option<Vec<crate::GridTrack>> = Some(vec![crate::GridTrack::Fr(1.0)]),
            GridRows => rows: Option<Vec<crate::GridTrack>> = Some(vec![crate::GridTrack::Px(40.0)]),
            GridColumnsUnsupported => columns_unsupported: Option<crate::GridTrackListUnsupported> =
                Some(crate::GridTrackListUnsupported::NestedRepeat),
            GridRowsUnsupported => rows_unsupported: Option<crate::GridTrackListUnsupported> =
                Some(crate::GridTrackListUnsupported::Subgrid),
            GridAutoColumns => auto_columns: Option<Vec<crate::GridTrack>> = Some(vec![crate::GridTrack::Auto]),
            GridAutoRows => auto_rows: Option<Vec<crate::GridTrack>> = Some(vec![crate::GridTrack::Px(32.0)]),
            GridAutoFlow => auto_flow: Option<crate::GridAutoFlow> = Some(crate::GridAutoFlow::RowDense),
            GridColumnsRepeat => columns_repeat: Option<crate::shared::Shared<crate::GridRepeatAuto>> =
                Some(crate::shared::Shared::new(crate::GridRepeatAuto {
                    tracks: vec![crate::GridTrack::Px(80.0)],
                    ..crate::GridRepeatAuto::default()
                })),
            GridRowsRepeat => rows_repeat: Option<crate::shared::Shared<crate::GridRepeatAuto>> =
                Some(crate::shared::Shared::new(crate::GridRepeatAuto {
                    tracks: vec![crate::GridTrack::Px(40.0)],
                    ..crate::GridRepeatAuto::default()
                })),
            GridColumnsSubgrid => columns_subgrid: bool = true,
            GridRowsSubgrid => rows_subgrid: bool = true,
            GridTemplateAreas => template_areas: Option<crate::GridTemplateAreas> = Some(crate::GridTemplateAreas {
                cells: vec![vec!["main".into()]],
            }),
            GridColumnLineNames => column_line_names: Option<Vec<Vec<String>>> = Some(vec![vec!["start".into()]]),
            GridRowLineNames => row_line_names: Option<Vec<Vec<String>>> = Some(vec![vec!["top".into()]]),
        }
        nested {
            placement {
                GridColumnStart => column_start: crate::GridLine = crate::GridLine::Index(1),
                GridColumnEnd => column_end: crate::GridLine = crate::GridLine::Span(2),
                GridRowStart => row_start: crate::GridLine = crate::GridLine::Index(1),
                GridRowEnd => row_end: crate::GridLine = crate::GridLine::Index(3),
                GridArea => area: Option<String> = Some("header".into()),
            }
        }
    }

    group border_sides {
        fields {
            BorderTopWidth => top_width: Option<f32> = Some(1.0),
            BorderRightWidth => right_width: Option<f32> = Some(2.0),
            BorderBottomWidth => bottom_width: Option<f32> = Some(3.0),
            BorderLeftWidth => left_width: Option<f32> = Some(4.0),
            BorderTopStyle => top_style: Option<crate::BorderStyle> = Some(crate::BorderStyle::Solid),
            BorderRightStyle => right_style: Option<crate::BorderStyle> = Some(crate::BorderStyle::Dashed),
            BorderBottomStyle => bottom_style: Option<crate::BorderStyle> = Some(crate::BorderStyle::Dotted),
            BorderLeftStyle => left_style: Option<crate::BorderStyle> = Some(crate::BorderStyle::Solid),
        }
        nested {}
    }

    group container {
        fields {
            ContainerType => container_type: crate::ContainerType = crate::ContainerType::InlineSize,
            ContainerName => container_name: Vec<String> = vec!["sidebar".into()],
        }
        nested {}
    }

    group paint {
        fields {
            BackgroundMaterial => background_material: Option<crate::MaterialRef> = Some(sample_material_ref()),
            BorderMaterial => border_material: Option<crate::MaterialRef> = Some(sample_material_ref()),
            TextMaterial => text_material: Option<crate::MaterialRef> = Some(sample_material_ref()),
            Visibility => visibility: Option<crate::VisibilitySpec> = Some(crate::VisibilitySpec::Hidden),
            BorderRadii => border_radii: Option<[crate::LengthSpec; 4]> = Some([crate::LengthSpec::Px(6.0); 4]),
            BoxShadows => box_shadows: Vec<crate::BoxShadowSpec> = vec![crate::BoxShadowSpec {
                offset_x: 0.0,
                offset_y: 4.0,
                blur_radius: 12.0,
                spread_radius: 0.0,
                color: sample_paint_color(),
                inset: false,
            }],
            Elevation => elevation: Option<crate::ElevationRole> = Some(crate::ElevationRole::Surface),
            Scrollbar => scrollbar: Option<crate::ScrollbarSkin> = Some(crate::ScrollbarSkin {
                thickness: Some(8.0),
                ..crate::ScrollbarSkin::default()
            }),
            ScrollEdgeFade => scroll_edge_fade: Option<crate::ScrollEdgeFade> =
                Some(crate::ScrollEdgeFade::new(12.0, 12.0)),
            TextShadows => text_shadows: Vec<crate::TextShadowSpec> = vec![crate::TextShadowSpec {
                offset_x: 1.0,
                offset_y: 1.0,
                blur_radius: 2.0,
                color: sample_paint_color(),
            }],
            TextStroke => text_stroke: Option<crate::TextStrokeSpec> = Some(crate::TextStrokeSpec {
                width: 1.0,
                color: Some(sample_paint_color()),
            }),
            PaintOrderStrokeFirst => paint_order_stroke_first: bool = true,
            MixBlend => mix_blend: crate::MixBlendMode = crate::MixBlendMode::Multiply,
            WideFilterBlur => wide_filter_blur: bool = true,
            MotionShutter => motion_shutter: f32 = 0.5,
            UnsupportedBorderImage => unsupported_border_image: bool = true,
            BorderImage => border_image: Option<crate::BorderImageSpec> =
                Some(crate::BorderImageSpec::from_source(crate::BackgroundImage::url(
                    "nana://res/frame.png",
                ))),
            BackgroundLayers => background_layers: Vec<crate::BackgroundImage> =
                vec![crate::BackgroundImage::url("nana://res/bg.png")],
            BackgroundSizeList => background_size_list: Vec<crate::BackgroundImageFit> =
                vec![crate::BackgroundImageFit::Cover],
            BackgroundSizeLengths => background_size_lengths: Vec<(Option<crate::LengthSpec>, Option<crate::LengthSpec>)> =
                vec![(Some(crate::LengthSpec::Px(64.0)), Some(crate::LengthSpec::Px(64.0)))],
            BackgroundPositionList => background_position_list: Vec<crate::BackgroundPosition> =
                vec![crate::BackgroundPosition::default()],
            BackgroundRepeatList => background_repeat_list: Vec<crate::BackgroundRepeat> =
                vec![crate::BackgroundRepeat::NoRepeat],
            ContentImage => content_image: Option<crate::BackgroundImage> =
                Some(crate::BackgroundImage::url("nana://res/content.png")),
            SkippedReplaced => skipped_replaced: Option<String> = Some("video".into()),
            ObjectFit => object_fit: Option<crate::BackgroundImageFit> =
                Some(crate::BackgroundImageFit::Contain),
            ObjectPosition => object_position: Option<crate::BackgroundPosition> =
                Some(crate::BackgroundPosition::default()),
            Mask => mask: Option<crate::MaskImage> = Some(crate::MaskImage::Url("nana://res/mask.png".into())),
            ClipPath => clip_path: Option<crate::ClipPath> = Some(crate::ClipPath::Inset(crate::ClipInset {
                top: crate::LengthSpec::Px(4.0),
                right: crate::LengthSpec::Px(4.0),
                bottom: crate::LengthSpec::Px(4.0),
                left: crate::LengthSpec::Px(4.0),
                round: Some(crate::LengthSpec::Px(2.0)),
            })),
            Filter => filter: Option<crate::ColorFilter> = Some(crate::ColorFilter {
                blur_radius: 4.0,
                ..crate::ColorFilter::default()
            }),
            BackdropFilter => backdrop_filter: Option<crate::BackdropFilter> = Some(crate::BackdropFilter {
                blur_radius: 8.0,
                saturate: 1.2,
            }),
        }
        nested {
            outline {
                OutlineWidth => width: f32 = 2.0,
                OutlineStyle => style: crate::OutlineStyle = crate::OutlineStyle::Solid,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{ElevationRole, LayoutStyle, LengthSpec};

    #[test]
    fn shared_group_noop_clear_does_not_materialize_arc() {
        let mut layout = LayoutStyle::default();
        assert!(layout.theme.is_shared_default());
        set::theme::padding_left(&mut layout, None);
        assert!(layout.theme.is_shared_default());

        set::paint::outline::width(&mut layout, 0.0);
        set::paint::elevation(&mut layout, None);
        assert!(layout.paint.is_shared_default());

        set::paint_colors::color(&mut layout, None);
        set::paint_colors::selection_background(&mut layout, None);
        assert!(layout.paint_colors.is_shared_default());

        set::transforms::preserve_3d(&mut layout, false);
        set::transforms::backface_visibility(&mut layout, crate::BackfaceVisibility::Visible);
        assert!(layout.transforms.is_shared_default());

        set::logical_edges::padding::inline_start(&mut layout, None);
        assert!(layout.logical_edges.is_shared_default());

        set::text_ext::letter_spacing(&mut layout, None);
        set::text_ext::placeholder_color(&mut layout, None);
        set::text_ext::text_typography::spacing_trim(&mut layout, None);
        assert!(layout.text_ext.is_shared_default());

        set::grid::placement::column_start(&mut layout, crate::GridLine::Auto);
        assert!(layout.grid.is_shared_default());

        set::border_sides::top_width(&mut layout, None);
        assert!(layout.border_sides.is_shared_default());

        set::container::container_type(&mut layout, crate::ContainerType::Normal);
        assert!(layout.container.is_shared_default());

        set::padding_left(&mut layout, Some(LengthSpec::Px(12.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn style_op_apply_noop_clear_does_not_materialize_shared_arcs() {
        let mut layout = LayoutStyle::default();
        let default = LayoutStyle::default();
        for path in LayoutStyle::overlay_leaf_paths() {
            let op = StyleOp::from_layout_path(&default, &path)
                .unwrap_or_else(|| panic!("missing StyleOp for leaf {path}"));
            op.apply(&mut layout);
        }
        assert!(layout.theme.is_shared_default());
        assert!(layout.paint_colors.is_shared_default());
        assert!(layout.transforms.is_shared_default());
        assert!(layout.logical_edges.is_shared_default());
        assert!(layout.text_ext.is_shared_default());
        assert!(layout.grid.is_shared_default());
        assert!(layout.paint.is_shared_default());
        assert!(layout.border_sides.is_shared_default());
        assert!(layout.container.is_shared_default());
    }

    #[test]
    fn style_op_table_matches_overlay_leaf_paths_one_to_one() {
        let samples = StyleOp::all_sample_variants();
        assert_eq!(samples.len(), STYLE_OP_COUNT);
        assert_eq!(STYLE_OP_COUNT, LayoutStyle::OVERLAY_LEAVES);

        let sample_paths: BTreeSet<&str> = samples.iter().map(StyleOp::field_path).collect();
        let overlay_paths = LayoutStyle::overlay_leaf_paths();
        let overlay_set: BTreeSet<&str> = overlay_paths.iter().map(String::as_str).collect();
        assert_eq!(sample_paths, overlay_set);

        let setter_paths: BTreeSet<&str> = samples.iter().map(StyleOp::setter_path).collect();
        assert_eq!(setter_paths.len(), STYLE_OP_COUNT);
    }

    #[test]
    fn every_sample_style_op_mutates_and_roundtrips_through_layout_style() {
        let default = LayoutStyle::default();
        for op in sample_style_ops() {
            let path = op.field_path();
            let mut via_apply = LayoutStyle::default();
            op.apply(&mut via_apply);
            assert_ne!(
                via_apply, default,
                "sample StyleOp for `{path}` must produce a non-default value"
            );

            let mut via_into = LayoutStyle::default();
            op.clone().apply_into(&mut via_into);
            assert_eq!(
                via_apply, via_into,
                "apply and apply_into diverged for `{path}`"
            );

            let extracted = StyleOp::from_layout_path(&via_apply, path)
                .unwrap_or_else(|| panic!("from_layout_path failed for `{path}`"));
            assert_eq!(extracted, op, "roundtrip mismatch for `{path}`");
        }
    }

    #[test]
    fn elevation_and_pseudo_element_setters_apply_cleanly() {
        let mut layout = LayoutStyle::default();
        set::paint::elevation(&mut layout, Some(ElevationRole::Overlay));
        set::text_ext::placeholder_color(&mut layout, Some([0.2, 0.3, 0.4, 0.9]));
        set::text_ext::placeholder_opacity(&mut layout, Some(0.5));
        set::paint_colors::selection_background(
            &mut layout,
            Some(crate::PaintColor::srgb([0.1, 0.2, 0.9, 0.3])),
        );
        set::paint_colors::selection_color(
            &mut layout,
            Some(crate::PaintColor::srgb([1.0, 1.0, 1.0, 1.0])),
        );
        assert_eq!(layout.paint.elevation, Some(ElevationRole::Overlay));
        assert_eq!(
            layout.text_ext.placeholder_color,
            Some([0.2, 0.3, 0.4, 0.9])
        );
        assert_eq!(layout.text_ext.placeholder_opacity, Some(0.5));
        assert_eq!(
            layout.paint_colors.selection_background,
            Some(crate::PaintColor::srgb([0.1, 0.2, 0.9, 0.3]))
        );
        assert_eq!(
            layout.paint_colors.selection_color,
            Some(crate::PaintColor::srgb([1.0, 1.0, 1.0, 1.0]))
        );
    }

    #[test]
    fn hot_style_patch_supports_both_ops_and_legacy_written() {
        let patch = HotStylePatch::from_ops(vec![
            StyleOp::Opacity(Some(0.75)),
            StyleOp::Elevation(Some(ElevationRole::Menu)),
            StyleOp::PaddingLeft(Some(LengthSpec::Px(16.0))),
        ]);
        let mut target = LayoutStyle::default();
        patch.apply(&mut target);
        assert_eq!(target.opacity, Some(0.75));
        assert_eq!(target.paint.elevation, Some(ElevationRole::Menu));
        assert_eq!(target.padding_left, Some(LengthSpec::Px(16.0)));

        let legacy_layout = LayoutStyle {
            order: 5,
            ..LayoutStyle::default()
        };
        let legacy = HotStylePatch::new(legacy_layout, vec!["order".into()]);
        legacy.apply(&mut target);
        assert_eq!(target.order, 5);
        assert_eq!(target.opacity, Some(0.75));
    }
}
