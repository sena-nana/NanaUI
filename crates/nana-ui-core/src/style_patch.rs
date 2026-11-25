//! Compiled style patches and direct field setters without runtime JSON.
//!
//! `nana-ui-view-codegen` emits compiled Rust statements calling functions in
//! [`set`] against a mutable [`crate::LayoutStyle`]. Every leaf path named by
//! [`crate::LayoutStyle::overlay_leaf_paths`] has a matching setter whose module
//! path mirrors the dotted field path (`theme.gap` -> `set::theme::gap`).

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

/// Dev-only data patch: a compiled `LayoutStyle` plus the dotted leaf paths
/// written by a rule's declarations. Never constructed in product builds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HotStylePatch {
    pub layout: crate::LayoutStyle,
    pub written: Vec<String>,
}

impl HotStylePatch {
    pub fn new(layout: crate::LayoutStyle, written: Vec<String>) -> Self {
        Self { layout, written }
    }

    pub fn apply(&self, target: &mut crate::LayoutStyle) {
        target.apply_written_leaves(&self.layout, &self.written);
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

/// Direct leaf setters on [`crate::LayoutStyle`] for compiled style patches.
pub mod set {
    use crate::box_layout::*;
    use crate::dynamic_layout::AdaptationProfile;
    use crate::{BorderStyle, LayoutStyle};

    macro_rules! top_setters {
        ($($name:ident : $ty:ty),* $(,)?) => {
            $(
                #[inline]
                pub fn $name(layout: &mut LayoutStyle, value: $ty) {
                    layout.$name = value;
                }
            )*
        };
    }

    top_setters! {
        layout_isolation: bool,
        direction: Option<FlexDirection>,
        dir: Option<DirSpec>,
        writing_mode: Option<WritingModeSpec>,
        flex_reverse: bool,
        order: i32,
        flex_wrap: FlexWrap,
        equal_wrap: bool,
        display: Option<DisplaySpec>,
        box_sizing: BoxSizing,
        position: PositionSpec,
        anchor: Option<std::sync::Arc<AnchorSpec>>,
        z_index: Option<i32>,
        isolation: bool,
        gap: Option<LengthSpec>,
        row_gap: Option<LengthSpec>,
        column_gap: Option<LengthSpec>,
        padding: Option<LengthSpec>,
        padding_top: Option<LengthSpec>,
        padding_right: Option<LengthSpec>,
        padding_bottom: Option<LengthSpec>,
        padding_left: Option<LengthSpec>,
        margin: Option<LengthSpec>,
        margin_top: Option<LengthSpec>,
        margin_right: Option<LengthSpec>,
        margin_bottom: Option<LengthSpec>,
        margin_left: Option<LengthSpec>,
        offset_top: Option<LengthSpec>,
        offset_right: Option<LengthSpec>,
        offset_bottom: Option<LengthSpec>,
        offset_left: Option<LengthSpec>,
        width: Option<LengthSpec>,
        height: Option<LengthSpec>,
        min_width: Option<LengthSpec>,
        max_width: Option<LengthSpec>,
        min_height: Option<LengthSpec>,
        max_height: Option<LengthSpec>,
        allow_shrink: bool,
        align_items: AlignSpec,
        align_self: Option<AlignSpec>,
        align_content: JustifySpec,
        justify_content: JustifySpec,
        justify_items: Option<AlignSpec>,
        justify_self: Option<AlignSpec>,
        flex_grow: Option<f32>,
        flex_shrink: Option<f32>,
        flex_basis: Option<LengthSpec>,
        adaptation: Option<std::sync::Arc<AdaptationProfile>>,
        overflow_x: OverflowSpec,
        overflow_y: OverflowSpec,
        text_overflow_ellipsis: bool,
        pointer_events: Option<PointerEventsSpec>,
        hit_test: Option<HitTestBehavior>,
        inert: bool,
        cursor: Option<CursorSpec>,
        user_select: Option<UserSelectSpec>,
        white_space: WhiteSpaceSpec,
        aspect_ratio: Option<f32>,
        text_align: TextAlignSpec,
        font_size: Option<f32>,
        font_weight: Option<u16>,
        font_italic: Option<bool>,
        line_height: Option<LineHeightSpec>,
        hidden: bool,
        opacity: Option<f32>,
        border_radius: Option<f32>,
        border_width: Option<f32>,
        border_style: Option<BorderStyle>,
    }

    macro_rules! shared_group_setters {
        ($group:ident { $($name:ident : $ty:ty),* $(,)? }) => {
            $(
                #[inline]
                pub fn $name(layout: &mut super::LayoutStyle, value: $ty) {
                    if layout.$group.$name == value {
                        return;
                    }
                    layout.$group.$name = value;
                }
            )*
        };
        ($group:ident . $sub:ident { $($name:ident : $ty:ty),* $(,)? }) => {
            $(
                #[inline]
                pub fn $name(layout: &mut super::super::LayoutStyle, value: $ty) {
                    if layout.$group.$sub.$name == value {
                        return;
                    }
                    layout.$group.$sub.$name = value;
                }
            )*
        };
    }

    pub mod theme {
        use crate::ThemeExpr;

        shared_group_setters!(theme {
            gap: Option<ThemeExpr>,
            row_gap: Option<ThemeExpr>,
            column_gap: Option<ThemeExpr>,
            padding: Option<ThemeExpr>,
            padding_top: Option<ThemeExpr>,
            padding_right: Option<ThemeExpr>,
            padding_bottom: Option<ThemeExpr>,
            padding_left: Option<ThemeExpr>,
            margin: Option<ThemeExpr>,
            margin_top: Option<ThemeExpr>,
            margin_right: Option<ThemeExpr>,
            margin_bottom: Option<ThemeExpr>,
            margin_left: Option<ThemeExpr>,
            offset_top: Option<ThemeExpr>,
            offset_right: Option<ThemeExpr>,
            offset_bottom: Option<ThemeExpr>,
            offset_left: Option<ThemeExpr>,
            width: Option<ThemeExpr>,
            height: Option<ThemeExpr>,
            min_width: Option<ThemeExpr>,
            max_width: Option<ThemeExpr>,
            min_height: Option<ThemeExpr>,
            max_height: Option<ThemeExpr>,
            flex_basis: Option<ThemeExpr>,
            font_size: Option<ThemeExpr>,
            font_weight: Option<ThemeExpr>,
            line_height: Option<ThemeExpr>,
            letter_spacing: Option<ThemeExpr>,
            border_width: Option<ThemeExpr>,
            border_top_width: Option<ThemeExpr>,
            border_right_width: Option<ThemeExpr>,
            border_bottom_width: Option<ThemeExpr>,
            border_left_width: Option<ThemeExpr>,
            border_radius: Option<ThemeExpr>,
            scrollbar_room: Option<ThemeExpr>,
        });
    }

    pub mod paint_colors {
        use crate::PaintColor;

        shared_group_setters!(paint_colors {
            color: Option<PaintColor>,
            background: Option<PaintColor>,
            border: Option<PaintColor>,
            border_top: Option<PaintColor>,
            border_right: Option<PaintColor>,
            border_bottom: Option<PaintColor>,
            border_left: Option<PaintColor>,
            outline: Option<PaintColor>,
            selection_background: Option<PaintColor>,
            selection_color: Option<PaintColor>,
        });
    }

    pub mod transforms {
        use crate::box_layout::*;
        use crate::{BackfaceVisibility, TransformBox, TransformOrigin};

        shared_group_setters!(transforms {
            transform: Option<PaintTransform>,
            transform_3d: Option<PaintMat4>,
            unsupported_transform: Option<String>,
            origin: Option<TransformOrigin>,
            css_perspective: Option<f32>,
            perspective_origin: Option<TransformOrigin>,
            preserve_3d: bool,
            backface_visibility: BackfaceVisibility,
            transform_box: TransformBox,
        });
    }

    pub mod logical_edges {
        macro_rules! logical_edges_module {
            ($mod_name:ident) => {
                pub mod $mod_name {
                    use crate::LengthSpec;

                    shared_group_setters!(logical_edges.$mod_name {
                        inline_start: Option<LengthSpec>,
                        inline_end: Option<LengthSpec>,
                        block_start: Option<LengthSpec>,
                        block_end: Option<LengthSpec>,
                        phys_top: Option<LengthSpec>,
                        phys_right: Option<LengthSpec>,
                        phys_bottom: Option<LengthSpec>,
                        phys_left: Option<LengthSpec>,
                        generations: [u32; 8],
                        next_gen: u32,
                        derived: u8,
                    });
                }
            };
        }

        logical_edges_module!(padding);
        logical_edges_module!(margin);
        logical_edges_module!(inset);
    }

    pub mod text_ext {
        use crate::box_layout::*;
        use crate::typography::{
            DominantBaselineSpec, FontKerningSpec, FontVariationSetting, HyphensSpec,
            LineBreakSpec, TextEmphasisColor, TextEmphasisPosition, TextEmphasisStyle,
        };

        shared_group_setters!(text_ext {
            font_family: Option<String>,
            letter_spacing: Option<f32>,
            line_clamp: Option<u16>,
            word_break: Option<WordBreakSpec>,
            overflow_wrap: Option<OverflowWrapSpec>,
            text_decoration: Option<TextDecorationLine>,
            text_emphasis_style: Option<TextEmphasisStyle>,
            text_emphasis_position: Option<TextEmphasisPosition>,
            text_emphasis_color: Option<TextEmphasisColor>,
            font_features: Option<Vec<FontFeatureSetting>>,
            font_variation_settings: Option<Vec<FontVariationSetting>>,
            font_kerning: Option<FontKerningSpec>,
            line_break: Option<LineBreakSpec>,
            hyphens: Option<HyphensSpec>,
            unsupported_font_variation: bool,
            unsupported_writing_mode: bool,
            text_orientation: Option<TextOrientationSpec>,
            dominant_baseline: Option<DominantBaselineSpec>,
            vertical_align: VerticalAlignSpec,
            placeholder_color: Option<[f32; 4]>,
            placeholder_opacity: Option<f32>,
        });

        pub mod text_typography {
            use crate::typography::{
                TextAutospaceSpec, TextJustifySpec, TextSpacingTrimSpec, TextWrapStyleSpec,
            };

            shared_group_setters!(text_ext.text_typography {
                spacing_trim: Option<TextSpacingTrimSpec>,
                autospace: Option<TextAutospaceSpec>,
                justify: Option<TextJustifySpec>,
                wrap_style: Option<TextWrapStyleSpec>,
                justify_lines: Option<bool>,
            });
        }
    }

    pub mod grid {
        use crate::shared::Shared;
        use crate::{
            GridAutoFlow, GridRepeatAuto, GridTemplateAreas, GridTrack, GridTrackListUnsupported,
        };

        shared_group_setters!(grid {
            columns: Option<Vec<GridTrack>>,
            rows: Option<Vec<GridTrack>>,
            columns_unsupported: Option<GridTrackListUnsupported>,
            rows_unsupported: Option<GridTrackListUnsupported>,
            auto_columns: Option<Vec<GridTrack>>,
            auto_rows: Option<Vec<GridTrack>>,
            auto_flow: Option<GridAutoFlow>,
            columns_repeat: Option<Shared<GridRepeatAuto>>,
            rows_repeat: Option<Shared<GridRepeatAuto>>,
            columns_subgrid: bool,
            rows_subgrid: bool,
            template_areas: Option<GridTemplateAreas>,
            column_line_names: Option<Vec<Vec<String>>>,
            row_line_names: Option<Vec<Vec<String>>>,
        });

        pub mod placement {
            use crate::GridLine;

            shared_group_setters!(grid.placement {
                column_start: GridLine,
                column_end: GridLine,
                row_start: GridLine,
                row_end: GridLine,
                area: Option<String>,
            });
        }
    }

    pub mod border_sides {
        use crate::BorderStyle;

        shared_group_setters!(border_sides {
            top_width: Option<f32>,
            right_width: Option<f32>,
            bottom_width: Option<f32>,
            left_width: Option<f32>,
            top_style: Option<BorderStyle>,
            right_style: Option<BorderStyle>,
            bottom_style: Option<BorderStyle>,
            left_style: Option<BorderStyle>,
        });
    }

    pub mod container {
        use crate::box_layout::ContainerType;

        shared_group_setters!(container {
            container_type: ContainerType,
            container_name: Vec<String>,
        });
    }

    pub mod paint {
        use crate::box_layout::*;
        use crate::{MaterialRef, ScrollEdgeFade, ScrollbarSkin};

        shared_group_setters!(paint {
            background_material: Option<MaterialRef>,
            border_material: Option<MaterialRef>,
            text_material: Option<MaterialRef>,
            visibility: Option<VisibilitySpec>,
            border_radii: Option<[LengthSpec; 4]>,
            box_shadows: Vec<BoxShadowSpec>,
            scrollbar: Option<ScrollbarSkin>,
            scroll_edge_fade: Option<ScrollEdgeFade>,
            text_shadows: Vec<TextShadowSpec>,
            text_stroke: Option<TextStrokeSpec>,
            paint_order_stroke_first: bool,
            mix_blend: MixBlendMode,
            wide_filter_blur: bool,
            motion_shutter: f32,
            unsupported_border_image: bool,
            border_image: Option<BorderImageSpec>,
            background_layers: Vec<BackgroundImage>,
            background_size_list: Vec<BackgroundImageFit>,
            background_size_lengths: Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
            background_position_list: Vec<BackgroundPosition>,
            background_repeat_list: Vec<BackgroundRepeat>,
            content_image: Option<BackgroundImage>,
            skipped_replaced: Option<String>,
            object_fit: Option<BackgroundImageFit>,
            object_position: Option<BackgroundPosition>,
            mask: Option<MaskImage>,
            clip_path: Option<ClipPath>,
            filter: Option<ColorFilter>,
            backdrop_filter: Option<BackdropFilter>,
        });

        pub mod outline {
            use crate::{LayoutStyle, OutlineStyle};

            #[inline]
            pub fn width(layout: &mut LayoutStyle, value: f32) {
                if layout.paint.outline.width == value {
                    return;
                }
                layout.paint.outline.width = value;
            }
            #[inline]
            pub fn style(layout: &mut LayoutStyle, value: OutlineStyle) {
                if layout.paint.outline.style == value {
                    return;
                }
                layout.paint.outline.style = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayoutStyle, LengthSpec};

    #[test]
    fn shared_group_noop_clear_does_not_materialize_arc() {
        let mut layout = LayoutStyle::default();
        assert!(layout.theme.is_shared_default());
        set::theme::padding_left(&mut layout, None);
        assert!(layout.theme.is_shared_default());

        set::paint::outline::width(&mut layout, 0.0);
        assert!(layout.paint.is_shared_default());

        set::transforms::preserve_3d(&mut layout, false);
        assert!(layout.transforms.is_shared_default());
        set::logical_edges::padding::inline_start(&mut layout, None);
        assert!(layout.logical_edges.is_shared_default());
        set::text_ext::letter_spacing(&mut layout, None);
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
}
