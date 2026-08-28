//! Consuming builders for every [`LayoutStyle`] / [`PaintStyle`] field.
//!
//! This is the L3 layout API. L1 CSS adapters write the same fields after parse;
//! they do not invent a second model.

use crate::box_layout::{
    AlignSpec, BackdropFilter, BackgroundImage, BackgroundImageFit, BackgroundPosition,
    BackgroundRepeat, BorderImageSpec, BorderStyle, BoxShadowSpec, BoxSizing, ClearSpec, ClipPath,
    ColorFilter, DirSpec, DisplaySpec, FlexDirection, FlexWrap, FloatSpec, FontFeatureSetting,
    GridAutoFlow, GridPlacement, GridRepeatAuto, GridTemplateAreas, GridTrack,
    GridTrackListUnsupported, JustifySpec, LayoutStyle, LengthSpec, LineHeightSpec,
    LogicalInlineEdges, MaskImage, MixBlendMode, OutlineSpec, OverflowSpec, OverflowWrapSpec,
    PaintMat4, PaintStyle, PaintTransform, PointerEventsSpec, PositionSpec, TextAlignSpec,
    TextDecorationLine, TextShadowSpec, TransformBox, TransformOrigin, VisibilitySpec,
    WhiteSpaceSpec, WordBreakSpec,
};

macro_rules! set_opt {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        $(
            #[inline]
            pub fn $name(mut self, value: $ty) -> Self {
                self.$name = Some(value);
                self
            }
        )+
    };
}

macro_rules! set_val {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        $(
            #[inline]
            pub fn $name(mut self, value: $ty) -> Self {
                self.$name = value;
                self
            }
        )+
    };
}

impl PaintStyle {
    set_opt! {
        visibility: VisibilitySpec,
        border_radii: [LengthSpec; 4],
        text_shadow: TextShadowSpec,
        border_image: BorderImageSpec,
        background_image: BackgroundImage,
        content_image: BackgroundImage,
        skipped_replaced: String,
        object_fit: BackgroundImageFit,
        object_position: BackgroundPosition,
        mask: MaskImage,
        clip_path: ClipPath,
        filter: ColorFilter,
        backdrop_filter: BackdropFilter,
    }

    set_val! {
        box_shadows: Vec<BoxShadowSpec>,
        outline: OutlineSpec,
        mix_blend: MixBlendMode,
        unsupported_border_image: bool,
        background_layers: Vec<BackgroundImage>,
        background_size_list: Vec<BackgroundImageFit>,
        background_size_lengths: Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
        background_position_list: Vec<BackgroundPosition>,
        background_repeat_list: Vec<BackgroundRepeat>,
    }
}

impl LayoutStyle {
    set_opt! {
        direction: FlexDirection,
        dir: DirSpec,
        display: DisplaySpec,
        z_index: i32,
        transform: PaintTransform,
        transform_3d: PaintMat4,
        unsupported_transform: String,
        transform_origin: TransformOrigin,
        css_perspective: f32,
        gap: LengthSpec,
        row_gap: LengthSpec,
        column_gap: LengthSpec,
        padding: LengthSpec,
        padding_top: LengthSpec,
        padding_right: LengthSpec,
        padding_bottom: LengthSpec,
        padding_left: LengthSpec,
        margin: LengthSpec,
        margin_top: LengthSpec,
        margin_right: LengthSpec,
        margin_bottom: LengthSpec,
        margin_left: LengthSpec,
        offset_top: LengthSpec,
        offset_right: LengthSpec,
        offset_bottom: LengthSpec,
        offset_left: LengthSpec,
        width: LengthSpec,
        height: LengthSpec,
        min_width: LengthSpec,
        max_width: LengthSpec,
        min_height: LengthSpec,
        max_height: LengthSpec,
        align_self: AlignSpec,
        justify_items: AlignSpec,
        justify_self: AlignSpec,
        flex_grow: f32,
        flex_shrink: f32,
        flex_basis: LengthSpec,
        line_clamp: u16,
        pointer_events: PointerEventsSpec,
        word_break: WordBreakSpec,
        overflow_wrap: OverflowWrapSpec,
        aspect_ratio: f32,
        font_size: f32,
        font_weight: u16,
        font_italic: bool,
        font_family: String,
        line_height: LineHeightSpec,
        letter_spacing: f32,
        color: [f32; 4],
        text_decoration: TextDecorationLine,
        font_features: Vec<FontFeatureSetting>,
        placeholder_color: [f32; 4],
        placeholder_opacity: f32,
        grid_columns: Vec<GridTrack>,
        grid_rows: Vec<GridTrack>,
        grid_columns_unsupported: GridTrackListUnsupported,
        grid_rows_unsupported: GridTrackListUnsupported,
        grid_auto_columns: Vec<GridTrack>,
        grid_auto_rows: Vec<GridTrack>,
        grid_auto_flow: GridAutoFlow,
        grid_columns_repeat: GridRepeatAuto,
        grid_rows_repeat: GridRepeatAuto,
        grid_template_areas: GridTemplateAreas,
        grid_column_line_names: Vec<Vec<String>>,
        grid_row_line_names: Vec<Vec<String>>,
        opacity: f32,
        background: [f32; 4],
        border_radius: f32,
        border_width: f32,
        border_top_width: f32,
        border_right_width: f32,
        border_bottom_width: f32,
        border_left_width: f32,
        border_color: [f32; 4],
        border_top_color: [f32; 4],
        border_right_color: [f32; 4],
        border_bottom_color: [f32; 4],
        border_left_color: [f32; 4],
        border_style: BorderStyle,
        border_top_style: BorderStyle,
        border_right_style: BorderStyle,
        border_bottom_style: BorderStyle,
        border_left_style: BorderStyle,
    }

    set_val! {
        unsupported_writing_mode: bool,
        flex_reverse: bool,
        order: i32,
        flex_wrap: FlexWrap,
        box_sizing: BoxSizing,
        position: PositionSpec,
        isolation: bool,
        transform_box: TransformBox,
        preserve_3d: bool,
        logical_padding: LogicalInlineEdges,
        logical_margin: LogicalInlineEdges,
        logical_inset: LogicalInlineEdges,
        allow_shrink: bool,
        align_items: AlignSpec,
        align_content: JustifySpec,
        justify_content: JustifySpec,
        overflow_x: OverflowSpec,
        overflow_y: OverflowSpec,
        text_overflow_ellipsis: bool,
        white_space_nowrap: bool,
        white_space: WhiteSpaceSpec,
        text_align: TextAlignSpec,
        float: FloatSpec,
        clear: ClearSpec,
        unsupported_font_variation: bool,
        grid_placement: GridPlacement,
        hidden: bool,
        paint: PaintStyle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_layout::{
        AlignSpec, BackgroundImage, BackgroundImageFit, BorderImageSlice, BorderImageSpec,
        BorderStyle, BoxShadowSpec, BoxSizing, ClearSpec, ClipInset, ClipPath, DirSpec,
        DisplaySpec, FlexDirection, FlexWrap, FloatSpec, GridAutoFlow, GridLine, GridPlacement,
        GridTrack, JustifySpec, LengthSpec, MixBlendMode, OverflowSpec, PaintTransform,
        PointerEventsSpec, PositionSpec, TextAlignSpec, TransformBox, VisibilitySpec,
        WhiteSpaceSpec,
    };

    #[test]
    fn layout_style_builders_cover_declared_fields() {
        let styled = LayoutStyle::default()
            .direction(FlexDirection::Row)
            .dir(DirSpec::Rtl)
            .unsupported_writing_mode(true)
            .flex_reverse(true)
            .order(-1)
            .flex_wrap(FlexWrap::Wrap)
            .display(DisplaySpec::Grid)
            .box_sizing(BoxSizing::ContentBox)
            .position(PositionSpec::Relative)
            .z_index(3)
            .isolation(true)
            .transform(PaintTransform::default())
            .unsupported_transform("skew".into())
            .transform_origin(TransformOrigin::default())
            .transform_box(TransformBox::ContentBox)
            .css_perspective(400.0)
            .preserve_3d(true)
            .gap(LengthSpec::Px(8.0))
            .row_gap(LengthSpec::Px(4.0))
            .column_gap(LengthSpec::Px(6.0))
            .padding(LengthSpec::Px(1.0))
            .padding_top(LengthSpec::Px(2.0))
            .padding_right(LengthSpec::Px(3.0))
            .padding_bottom(LengthSpec::Px(4.0))
            .padding_left(LengthSpec::Px(5.0))
            .margin(LengthSpec::Px(1.0))
            .margin_top(LengthSpec::Px(2.0))
            .margin_right(LengthSpec::Px(3.0))
            .margin_bottom(LengthSpec::Px(4.0))
            .margin_left(LengthSpec::Px(5.0))
            .offset_top(LengthSpec::Px(1.0))
            .offset_right(LengthSpec::Px(2.0))
            .offset_bottom(LengthSpec::Px(3.0))
            .offset_left(LengthSpec::Px(4.0))
            .width(LengthSpec::Px(100.0))
            .height(LengthSpec::Px(50.0))
            .min_width(LengthSpec::Px(10.0))
            .max_width(LengthSpec::Px(200.0))
            .min_height(LengthSpec::Px(8.0))
            .max_height(LengthSpec::Px(80.0))
            .allow_shrink(true)
            .align_items(AlignSpec::Center)
            .align_self(AlignSpec::End)
            .align_content(JustifySpec::SpaceBetween)
            .justify_content(JustifySpec::End)
            .justify_items(AlignSpec::Stretch)
            .justify_self(AlignSpec::Center)
            .flex_grow(1.0)
            .flex_shrink(0.0)
            .flex_basis(LengthSpec::Px(40.0))
            .overflow_x(OverflowSpec::Hidden)
            .overflow_y(OverflowSpec::Auto)
            .text_overflow_ellipsis(true)
            .line_clamp(2)
            .pointer_events(PointerEventsSpec::None)
            .white_space_nowrap(true)
            .white_space(WhiteSpaceSpec::Pre)
            .aspect_ratio(1.5)
            .text_align(TextAlignSpec::Center)
            .float(FloatSpec::Left)
            .clear(ClearSpec::Both)
            .font_size(16.0)
            .font_weight(700)
            .font_italic(true)
            .font_family("Inter".into())
            .letter_spacing(0.2)
            .color([1.0, 0.0, 0.0, 1.0])
            .font_features(Vec::new())
            .unsupported_font_variation(true)
            .placeholder_color([0.0, 0.0, 0.0, 0.5])
            .placeholder_opacity(0.4)
            .grid_columns(vec![GridTrack::Px(80.0)])
            .grid_rows(vec![GridTrack::Fr(1.0)])
            .grid_auto_flow(GridAutoFlow::Column)
            .grid_placement(GridPlacement {
                column_start: GridLine::Index(1),
                column_end: GridLine::Auto,
                row_start: GridLine::Auto,
                row_end: GridLine::Auto,
                area: None,
            })
            .hidden(true)
            .opacity(0.5)
            .background([0.1, 0.2, 0.3, 1.0])
            .border_radius(8.0)
            .border_width(1.0)
            .border_top_width(2.0)
            .border_right_width(3.0)
            .border_bottom_width(4.0)
            .border_left_width(5.0)
            .border_color([0.0, 0.0, 0.0, 1.0])
            .border_style(BorderStyle::Solid)
            .paint(
                PaintStyle::default()
                    .visibility(VisibilitySpec::Hidden)
                    .box_shadows(vec![BoxShadowSpec {
                        offset_x: 1.0,
                        offset_y: 2.0,
                        blur_radius: 3.0,
                        spread_radius: 0.0,
                        color: [0.0, 0.0, 0.0, 0.4],
                        inset: false,
                    }])
                    .mix_blend(MixBlendMode::Multiply)
                    .background_image(BackgroundImage::url("tile.png"))
                    .object_fit(BackgroundImageFit::Cover)
                    .clip_path(ClipPath::Inset(ClipInset {
                        top: LengthSpec::Px(1.0),
                        right: LengthSpec::Px(2.0),
                        bottom: LengthSpec::Px(3.0),
                        left: LengthSpec::Px(4.0),
                        round: None,
                    }))
                    .skipped_replaced("iframe".into()),
            );

        assert_ne!(styled, LayoutStyle::default());
        assert_eq!(styled.direction, Some(FlexDirection::Row));
        assert_eq!(styled.display, Some(DisplaySpec::Grid));
        assert_eq!(styled.position, PositionSpec::Relative);
        assert_eq!(
            styled.grid_columns.as_deref(),
            Some(&[GridTrack::Px(80.0)][..])
        );
        assert_eq!(styled.paint.visibility, Some(VisibilitySpec::Hidden));
        assert_eq!(styled.paint.mix_blend, MixBlendMode::Multiply);
        assert_eq!(styled.paint.skipped_replaced.as_deref(), Some("iframe"));
    }

    #[test]
    fn paint_style_builders_round_trip() {
        let paint = PaintStyle::default()
            .visibility(VisibilitySpec::Hidden)
            .border_radii([LengthSpec::Px(1.0); 4])
            .box_shadows(vec![BoxShadowSpec {
                offset_x: 0.0,
                offset_y: 1.0,
                blur_radius: 2.0,
                spread_radius: 0.0,
                color: [0.0, 0.0, 0.0, 0.3],
                inset: false,
            }])
            .mix_blend(MixBlendMode::Screen)
            .unsupported_border_image(true)
            .border_image(BorderImageSpec {
                source: BackgroundImage::url("frame.png"),
                slice: [BorderImageSlice::Number(10.0); 4],
                fill: true,
            })
            .background_image(BackgroundImage::url("bg.png"))
            .content_image(BackgroundImage::url("img.png"))
            .object_fit(BackgroundImageFit::Contain)
            .clip_path(ClipPath::Inset(ClipInset {
                top: LengthSpec::Px(0.0),
                right: LengthSpec::Px(0.0),
                bottom: LengthSpec::Px(0.0),
                left: LengthSpec::Px(0.0),
                round: None,
            }));
        assert_ne!(paint, PaintStyle::default());
        assert_eq!(paint.visibility, Some(VisibilitySpec::Hidden));
        assert!(paint.unsupported_border_image);
    }
}
