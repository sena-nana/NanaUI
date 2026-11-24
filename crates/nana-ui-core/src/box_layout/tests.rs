use super::{
    BackgroundImage, BorderImageSlice, BorderImageSpec, BorderStyle, BoxShadowSpec, BoxSizing,
    DirSpec, DisplaySpec, FlexDirection, FontFeatureSetting, FontSizeContext, GridRepeatAuto,
    LayoutStyle, LengthSpec, LineHeightSpec, OverflowSpec, PaintMat4, PaintTransform, ParentBox,
    PointerEventsSpec, TEXT_APPROX_ASCENT_EM, TransformBox, TransformOrigin, VisibilitySpec,
    WordBreakSpec, WritingModeSpec, glyph_box_center_from_line_top, icon_y_on_text_glyph_center,
    text_line_box_height_px,
};
use crate::typography::{FontKerningSpec, FontVariationSetting, LineBreakSpec};

#[test]
fn scrollable_overflow_is_also_a_paint_clip() {
    assert!(OverflowSpec::Hidden.clips());
    assert!(OverflowSpec::Auto.clips());
    assert!(OverflowSpec::Scroll.clips());
    assert!(!OverflowSpec::Visible.clips());
}

#[test]
fn border_image_tiles_use_slice_and_clamp_to_box() {
    let spec = BorderImageSpec {
        source: BackgroundImage::url("frame.png"),
        slice: [BorderImageSlice::Number(30.0); 4],
        fill: true,
    };
    let tiles = spec.tiles(90.0, 90.0, 100.0, 80.0);
    let center = tiles
        .iter()
        .find(|tile| (tile.dest_x - 30.0).abs() < 0.01 && (tile.dest_y - 30.0).abs() < 0.01)
        .expect("fill center");
    assert!((center.dest_w - 40.0).abs() < 0.01);
    assert!((center.dest_h - 20.0).abs() < 0.01);
    assert!((center.u0 - 30.0 / 90.0).abs() < 0.01);

    let tight = spec.tiles(90.0, 90.0, 40.0, 40.0);
    let tl = tight
        .iter()
        .find(|tile| tile.dest_x.abs() < 0.01 && tile.dest_y.abs() < 0.01)
        .expect("tl");
    assert!((tl.dest_w - 20.0).abs() < 0.01);
    assert!((tl.dest_h - 20.0).abs() < 0.01);
}

#[test]
fn four_side_border_widths_and_none_style() {
    let layout = LayoutStyle::default()
        .border_top_width(1.0)
        .border_right_width(2.0)
        .border_bottom_width(3.0)
        .border_left_width(4.0)
        .border_top_color([1.0, 0.0, 0.0, 1.0])
        .border_right_color([0.0, 1.0, 0.0, 1.0])
        .border_bottom_color([0.0, 0.0, 1.0, 1.0])
        .border_left_color([1.0, 1.0, 0.0, 1.0]);
    let edges = layout.resolved_border_edges();
    assert!((edges.top - 1.0).abs() < f32::EPSILON);
    assert!((edges.right - 2.0).abs() < f32::EPSILON);
    assert!((edges.bottom - 3.0).abs() < f32::EPSILON);
    assert!((edges.left - 4.0).abs() < f32::EPSILON);
    assert!((layout.resolved_border_width() - 4.0).abs() < f32::EPSILON);
    let colors = layout.resolved_border_edge_colors();
    assert_eq!(colors[0], Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(colors[1], Some([0.0, 1.0, 0.0, 1.0]));
    assert_eq!(colors[2], Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(colors[3], Some([1.0, 1.0, 0.0, 1.0]));

    let none_left = LayoutStyle::default()
        .border_width(5.0)
        .border_left_style(BorderStyle::None);
    let used = none_left.resolved_border_edges();
    assert!((used.top - 5.0).abs() < f32::EPSILON);
    assert!(used.left.abs() < f32::EPSILON);
}

#[test]
fn dashed_dotted_paint_stroke_double_stays_closed() {
    let dashed = LayoutStyle {
        border_width: Some(4.0),
        border_style: Some(BorderStyle::Dashed),
        ..LayoutStyle::default()
    }
    .border_color([1.0, 0.0, 0.0, 1.0]);
    assert!((dashed.resolved_border_edges().top - 4.0).abs() < f32::EPSILON);
    assert!((dashed.paint_border_edges().top - 4.0).abs() < f32::EPSILON);
    assert_eq!(
        dashed.paint_border_style_codes(),
        [BorderStyle::SHADER_DASHED; 4]
    );
    assert!(dashed.paints_any_border());

    let dotted = LayoutStyle {
        border_width: Some(4.0),
        border_style: Some(BorderStyle::Dotted),
        ..LayoutStyle::default()
    }
    .border_color([1.0, 0.0, 0.0, 1.0]);
    assert!((dotted.paint_border_edges().top - 4.0).abs() < f32::EPSILON);
    assert_eq!(
        dotted.paint_border_style_codes(),
        [BorderStyle::SHADER_DOTTED; 4]
    );

    let double = LayoutStyle {
        border_width: Some(4.0),
        border_style: Some(BorderStyle::Unsupported),
        ..LayoutStyle::default()
    }
    .border_color([1.0, 0.0, 0.0, 1.0]);
    assert!((double.resolved_border_edges().top - 4.0).abs() < f32::EPSILON);
    assert!(double.paint_border_edges().is_zero());
    assert!(!double.paints_any_border());
}

#[test]
fn overflow_clip_box_opens_visible_axis() {
    let mut layout = LayoutStyle {
        overflow_x: OverflowSpec::Hidden,
        overflow_y: OverflowSpec::Visible,
        ..Default::default()
    };
    let (x, y, w, h) = layout.overflow_clip_box(10.0, 20.0, 30.0, 40.0).unwrap();
    assert!((x - 10.0).abs() < f32::EPSILON);
    assert!((w - 30.0).abs() < f32::EPSILON);
    assert!(y < 20.0);
    assert!(h > 40.0);

    layout.overflow_x = OverflowSpec::Visible;
    layout.overflow_y = OverflowSpec::Hidden;
    let (x, y, w, h) = layout.overflow_clip_box(10.0, 20.0, 30.0, 40.0).unwrap();
    assert!(x < 10.0);
    assert!(w > 30.0);
    assert!((y - 20.0).abs() < f32::EPSILON);
    assert!((h - 40.0).abs() < f32::EPSILON);

    layout.overflow_x = OverflowSpec::Visible;
    layout.overflow_y = OverflowSpec::Visible;
    assert!(layout.overflow_clip_box(0.0, 0.0, 10.0, 10.0).is_none());
}

#[test]
fn child_main_length_grows_to_fill() {
    let layout = LayoutStyle {
        flex_grow: Some(1.0),
        ..LayoutStyle::default()
    };
    assert_eq!(
        layout.child_main_length(FlexDirection::Row),
        Some(LengthSpec::Fill)
    );
}

#[test]
fn resolve_content_box_height_chain() {
    let shell = LayoutStyle {
        height: Some(LengthSpec::Fill),
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(800.0, 600.0);
    let box_ = shell.resolve_content_box(parent);
    assert_eq!(box_.height, Some(600.0));
}

#[test]
fn resolve_content_box_aspect_ratio_fills_auto_height() {
    let layout = LayoutStyle {
        width: Some(LengthSpec::Px(80.0)),
        aspect_ratio: Some(1.0),
        ..LayoutStyle::default()
    };
    let box_ = layout.resolve_content_box(ParentBox::from_viewport(400.0, 200.0));
    assert_eq!(box_.width, Some(80.0));
    assert_eq!(box_.height, Some(80.0));

    let wide = LayoutStyle {
        width: Some(LengthSpec::Px(160.0)),
        aspect_ratio: Some(16.0 / 9.0),
        ..LayoutStyle::default()
    };
    let box_ = wide.resolve_content_box(ParentBox::from_viewport(400.0, 200.0));
    assert!((box_.height.unwrap() - 90.0).abs() < 0.01);
}

#[test]
fn resolve_content_box_aspect_ratio_does_not_shrink_auto_width() {
    let layout = LayoutStyle {
        height: Some(LengthSpec::Px(80.0)),
        aspect_ratio: Some(1.0),
        ..LayoutStyle::default()
    };
    let cb = ParentBox::from_viewport(400.0, 200.0);
    let box_ = layout.resolve_content_box(cb);
    assert_eq!(
        box_.width,
        Some(400.0),
        "block width:auto used width is the CB, not height×ratio"
    );
    assert_eq!(box_.height, Some(80.0));

    let child = LayoutStyle {
        width: Some(LengthSpec::Percent(50.0)),
        ..LayoutStyle::default()
    };
    assert_eq!(
        child.resolve_content_box(box_).width,
        Some(200.0),
        "% children must resolve against the CB, not a shrink-wrapped 80"
    );
}

#[test]
fn resolve_content_box_keeps_declared_width_under_content_box() {
    let layout = LayoutStyle {
        width: Some(LengthSpec::Px(100.0)),
        padding: Some(LengthSpec::Px(10.0)),
        box_sizing: BoxSizing::ContentBox,
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(400.0, 200.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(
        box_.width,
        Some(100.0),
        "content-box width is content, not border"
    );
}

#[test]
fn resolve_content_box_fill_subtracts_chrome_under_content_box() {
    // Fill / 100% allocation is border-box; content area = allocation − pad − border.
    let layout = LayoutStyle {
        width: Some(LengthSpec::Fill),
        height: Some(LengthSpec::Fill),
        padding: Some(LengthSpec::Px(10.0)),
        border_width: Some(5.0),
        box_sizing: BoxSizing::ContentBox,
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(400.0, 200.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(
        box_.width,
        Some(370.0),
        "Fill border-box 400 − 20pad − 10border"
    );
    assert_eq!(
        box_.height,
        Some(170.0),
        "Fill border-box 200 − 20pad − 10border"
    );
}

#[test]
fn resolve_content_box_grow_auto_subtracts_chrome_under_content_box() {
    // flex-grow>0 with auto main: allocated share is border-box.
    let layout = LayoutStyle {
        flex_grow: Some(1.0),
        padding: Some(LengthSpec::Px(8.0)),
        border_width: Some(2.0),
        box_sizing: BoxSizing::ContentBox,
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(300.0, 150.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(box_.width, Some(280.0), "300 − 16pad − 4border");
    assert_eq!(box_.height, Some(130.0), "150 − 16pad − 4border");
}

#[test]
fn resolve_content_box_subtracts_padding_and_border_under_border_box() {
    let layout = LayoutStyle {
        width: Some(LengthSpec::Px(100.0)),
        padding: Some(LengthSpec::Px(10.0)),
        border_width: Some(5.0),
        box_sizing: BoxSizing::BorderBox,
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(400.0, 200.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(
        box_.width,
        Some(70.0),
        "border-box content = 100 - 20pad - 10border"
    );
}

#[test]
fn min_height_zero_does_not_inherit_parent_height() {
    let layout = LayoutStyle {
        min_height: Some(LengthSpec::Px(0.0)),
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(800.0, 600.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(
        box_.height, None,
        "min-height:0 only allows shrink; Fill comes from height/flex-grow"
    );
}

#[test]
fn min_height_zero_with_fill_height_still_chains() {
    let layout = LayoutStyle {
        height: Some(LengthSpec::Fill),
        min_height: Some(LengthSpec::Px(0.0)), // sentinel from CSS min-height:100%
        ..LayoutStyle::default()
    };
    let parent = ParentBox::from_viewport(800.0, 600.0);
    let box_ = layout.resolve_content_box(parent);
    assert_eq!(box_.height, Some(600.0));
}

#[test]
fn row_cross_gap_percent_falls_back_when_height_indefinite() {
    let layout = LayoutStyle {
        gap: Some(LengthSpec::Percent(10.0)),
        ..LayoutStyle::default()
    };
    // Auto-height wrap: content_h is 0 before shrink-to-fit — must not lock % to 0.
    let indefinite = ParentBox::new(Some(200.0), Some(0.0));
    assert_eq!(
        layout.cross_gap_against(FlexDirection::Row, indefinite),
        20.0
    );
    let definite = ParentBox::new(Some(200.0), Some(100.0));
    assert_eq!(layout.cross_gap_against(FlexDirection::Row, definite), 10.0);
}

#[test]
fn percent_resolve_px() {
    assert_eq!(
        LengthSpec::Percent(50.0).resolve_px(Some(400.0)),
        Some(200.0)
    );
}

#[test]
fn padding_margin_percent_resolves_against_containing_block_width() {
    let layout = LayoutStyle {
        padding: Some(LengthSpec::Percent(10.0)),
        margin_top: Some(LengthSpec::Percent(5.0)),
        margin_left: Some(LengthSpec::Px(8.0)),
        ..LayoutStyle::default()
    };
    let pad = layout.resolved_padding_against(Some(200.0));
    let margin = layout.resolved_margin_against(Some(200.0));
    assert_eq!(pad.top, 20.0);
    assert_eq!(pad.left, 20.0);
    assert_eq!(margin.top, 10.0);
    assert_eq!(margin.left, 8.0);
    assert!(layout.resolved_padding().is_zero());
}

#[test]
fn padding_em_default_api_stays_16px_fonts_uses_element_px() {
    let layout = LayoutStyle {
        padding: Some(LengthSpec::Em(1.0)),
        ..LayoutStyle::default()
    };
    let pad = layout.resolved_padding_against(None);
    assert_eq!(pad.left, 16.0);
    assert_eq!(pad.top, 16.0);
    let with_font = LayoutStyle {
        font_size: Some(32.0),
        padding: Some(LengthSpec::Em(1.0)),
        ..LayoutStyle::default()
    };
    assert_eq!(with_font.resolved_padding_against(None).left, 32.0);
    let pad32 = layout.resolved_padding_against_fonts(None, FontSizeContext::new(16.0, 32.0));
    assert_eq!(pad32.left, 32.0);
    assert_eq!(pad32.top, 32.0);
}

#[test]
fn resolve_inset_em_default_api_stays_16px_fonts_uses_element_px() {
    let spec = Some(LengthSpec::Em(1.0));
    assert_eq!(LayoutStyle::resolve_inset(spec, 200.0), Some(16.0));
    assert_eq!(
        LayoutStyle::resolve_inset_fonts(spec, 200.0, FontSizeContext::new(16.0, 32.0)),
        Some(32.0)
    );
}

#[test]
fn calc_percent_offset_resolve_px() {
    assert_eq!(
        LengthSpec::percent_offset(100.0, -40.0).resolve_px(Some(400.0)),
        Some(360.0)
    );
    assert_eq!(
        LengthSpec::percent_offset(50.0, 10.0).resolve_px(Some(200.0)),
        Some(110.0)
    );
}

#[test]
fn resolve_grid_minmax_nonzero_min_freezes() {
    use super::{GridTrack, resolve_grid_column_widths};
    let tracks = [
        GridTrack::MinMax {
            min_px: 400.0,
            fr: 1.0,
            max_px: None,
        },
        GridTrack::Fr(1.0),
    ];
    let w = resolve_grid_column_widths(&tracks, 600.0, 0.0);
    assert!((w[0] - 400.0).abs() < 0.01);
    assert!((w[1] - 200.0).abs() < 0.01);
}

#[test]
fn resolve_grid_auto_tracks_use_intrinsics_not_fr() {
    use super::{GridTrack, resolve_grid_column_widths, resolve_grid_track_sizes};
    let tracks = [
        GridTrack::Auto,
        GridTrack::Auto,
        GridTrack::MinMax {
            min_px: 0.0,
            fr: 1.0,
            max_px: None,
        },
    ];
    // Without intrinsics, auto → 0 and fr takes the rest.
    let bare = resolve_grid_column_widths(&tracks, 400.0, 12.0);
    assert!((bare[0] - 0.0).abs() < 0.01);
    assert!((bare[1] - 0.0).abs() < 0.01);
    assert!((bare[2] - 376.0).abs() < 0.01, "got {}", bare[2]);
    // With content contributions, auto keeps size; fr takes remainder.
    let sized = resolve_grid_track_sizes(&tracks, 400.0, 12.0, &[40.0, 80.0]);
    assert!((sized[0] - 40.0).abs() < 0.01);
    assert!((sized[1] - 80.0).abs() < 0.01);
    assert!((sized[2] - 256.0).abs() < 0.01, "got {}", sized[2]);
    assert_eq!(GridTrack::Auto.fr_weight(), None);
    assert_eq!(
        GridTrack::Auto.as_row_main_length(),
        super::LengthSpec::Shrink
    );
}

#[test]
fn resolve_grid_fractional_fr_weights() {
    use super::{GridTrack, resolve_grid_column_widths};
    let tracks = [GridTrack::Fr(1.0), GridTrack::Fr(1.5)];
    let w = resolve_grid_column_widths(&tracks, 500.0, 0.0);
    assert!((w[0] - 200.0).abs() < 0.01);
    assert!((w[1] - 300.0).abs() < 0.01);
}

#[test]
fn resolve_grid_multi_min_freeze_same_pass() {
    use super::{GridTrack, resolve_grid_column_widths};
    let tracks = [
        GridTrack::MinMax {
            min_px: 250.0,
            fr: 1.0,
            max_px: None,
        },
        GridTrack::MinMax {
            min_px: 250.0,
            fr: 1.0,
            max_px: None,
        },
        GridTrack::Fr(1.0),
    ];
    let w = resolve_grid_column_widths(&tracks, 600.0, 0.0);
    assert!((w[0] - 250.0).abs() < 0.01);
    assert!((w[1] - 250.0).abs() < 0.01);
    assert!((w[2] - 100.0).abs() < 0.01);
}

#[test]
fn resolve_grid_minmax_px_max_clamps() {
    use super::{GridTrack, resolve_grid_column_widths};
    let tracks = [
        GridTrack::MinMax {
            min_px: 50.0,
            fr: 1.0,
            max_px: Some(120.0),
        },
        GridTrack::Fr(1.0),
    ];
    let w = resolve_grid_column_widths(&tracks, 400.0, 0.0);
    assert!((w[0] - 120.0).abs() < 0.01);
    assert!((w[1] - 280.0).abs() < 0.01);
}

#[test]
fn resolve_grid_multi_max_freeze_same_pass() {
    use super::{GridTrack, resolve_grid_column_widths};
    let tracks = [
        GridTrack::MinMax {
            min_px: 0.0,
            fr: 1.0,
            max_px: Some(100.0),
        },
        GridTrack::MinMax {
            min_px: 0.0,
            fr: 1.0,
            max_px: Some(100.0),
        },
        GridTrack::Fr(1.0),
    ];
    let w = resolve_grid_column_widths(&tracks, 400.0, 0.0);
    assert!((w[0] - 100.0).abs() < 0.01);
    assert!((w[1] - 100.0).abs() < 0.01);
    assert!((w[2] - 200.0).abs() < 0.01);
}

#[test]
fn resolve_grid_rows_reuse_track_resolver_with_gap() {
    use super::{GridTrack, resolve_grid_column_widths};
    // height 400, gap 20 → free after 100px + 2×gap = 260 → two 1fr = 130
    let tracks = [GridTrack::Px(100.0), GridTrack::Fr(1.0), GridTrack::Fr(1.0)];
    let h = resolve_grid_column_widths(&tracks, 400.0, 20.0);
    assert!((h[0] - 100.0).abs() < 0.01);
    assert!((h[1] - 130.0).abs() < 0.01);
    assert!((h[2] - 130.0).abs() < 0.01);
}

#[test]
fn named_grid_line_nth_skips_first_occurrence() {
    let layout = LayoutStyle::default().grid_column_line_names(vec![
        vec!["foo".into()],
        vec!["foo".into()],
        vec!["foo".into()],
    ]);
    assert_eq!(layout.named_column_line("foo"), Some(1));
    assert_eq!(layout.named_column_line_nth("foo", 2), Some(2));
    assert_eq!(layout.named_column_line_nth("foo", 3), Some(3));
    assert_eq!(layout.named_column_line_after("foo", 1), Some(2));
    assert_eq!(layout.named_column_line_after("foo", 2), Some(3));
    assert_eq!(layout.named_column_line_after("foo", 3), None);
}

#[test]
fn omits_box_skips_display_none_not_visibility_hidden() {
    let none = LayoutStyle {
        display: Some(DisplaySpec::None),
        ..Default::default()
    };
    assert!(none.omits_box());

    let mut hidden = LayoutStyle::default();
    hidden.paint.visibility = Some(VisibilitySpec::Hidden);
    assert!(!hidden.omits_box());
    assert!(!hidden.is_paint_visible());
}

#[test]
fn pointer_events_none_is_not_hittable() {
    assert!(!PointerEventsSpec::None.hittable());
    assert!(PointerEventsSpec::Auto.hittable());
    assert_eq!(
        PointerEventsSpec::inherit_from(None, PointerEventsSpec::None),
        PointerEventsSpec::None
    );
    assert_eq!(
        PointerEventsSpec::inherit_from(Some(PointerEventsSpec::Auto), PointerEventsSpec::None),
        PointerEventsSpec::Auto
    );
}

#[test]
fn merge_line_name_pattern_copies_per_repetition() {
    let pattern = vec![vec!["mid".to_string()], vec!["end".to_string()]];
    let once = GridRepeatAuto::merge_line_name_pattern(&pattern, 1);
    assert_eq!(once, pattern);
    let twice = GridRepeatAuto::merge_line_name_pattern(&pattern, 2);
    assert_eq!(
        twice,
        vec![
            vec!["mid".to_string()],
            vec!["end".to_string(), "mid".to_string()],
            vec!["end".to_string()],
        ]
    );
    assert_eq!(
        twice
            .iter()
            .filter(|line| line.iter().any(|n| n == "mid"))
            .count(),
        2
    );
}

#[test]
fn glyph_box_center_sits_on_the_em_square_midline() {
    let font = 12.0;
    let line = text_line_box_height_px(font, Some(LineHeightSpec::Absolute(font)));
    assert!((line - font).abs() < f32::EPSILON);
    let center = glyph_box_center_from_line_top(line, font);
    let expected = font * TEXT_APPROX_ASCENT_EM - font * 0.5;
    assert!((center - expected).abs() < 1e-5);
    assert!((center - 3.6).abs() < 1e-5);
}

#[test]
fn icon_y_on_centered_text_shares_the_line_box_midline() {
    let y = icon_y_on_text_glyph_center(
        10.0,
        28.0,
        12.0,
        Some(LineHeightSpec::Absolute(12.0)),
        true,
        12.0,
    );
    assert!((y - 18.0).abs() < 1e-5);
}

#[test]
fn paint_transform_then_scales_translation_on_the_same_2x3() {
    let scale_x = PaintTransform {
        a: 0.5,
        ..PaintTransform::default()
    };
    let composed = scale_x.then(PaintTransform {
        e: 10.0,
        ..PaintTransform::default()
    });
    assert!((composed.a - 0.5).abs() < 1e-5);
    assert!((composed.e - 5.0).abs() < 1e-5);
    let world = composed.around_center(0.0, 0.0, 20.0, 20.0);
    assert_eq!(world.len(), 6);
}

#[test]
fn around_center_matches_50_percent_origin_not_zero_zero() {
    let rotate_90 = PaintTransform {
        a: 0.0,
        b: 1.0,
        c: -1.0,
        d: 0.0,
        ..PaintTransform::default()
    };
    let center = rotate_90.around_center(10.0, 20.0, 40.0, 20.0);
    let pct = rotate_90.around_origin(10.0, 20.0, 20.0, 10.0);
    let zero = rotate_90.around_origin(10.0, 20.0, 0.0, 0.0);
    assert_eq!(center, pct);
    assert_ne!(center, zero);
    assert_eq!(
        LayoutStyle::default()
            .transform(rotate_90)
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
        Some(center)
    );
    assert_eq!(
        LayoutStyle::default()
            .transform(rotate_90)
            .transform_origin(TransformOrigin {
                x: LengthSpec::Px(0.0),
                y: LengthSpec::Px(0.0),
            })
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
        Some(zero)
    );
    assert_eq!(
        LayoutStyle::default()
            .transform(rotate_90)
            .transform_origin(TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Percent(50.0),
            })
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
        Some(center)
    );
}

#[test]
fn transform_box_shifts_origin_onto_content_box() {
    let rotate_90 = PaintTransform {
        a: 0.0,
        b: 1.0,
        c: -1.0,
        d: 0.0,
        ..PaintTransform::default()
    };
    let origin = TransformOrigin {
        x: LengthSpec::Px(0.0),
        y: LengthSpec::Px(0.0),
    };
    let mut style = LayoutStyle::default()
        .transform(rotate_90)
        .transform_origin(origin)
        .padding(LengthSpec::Px(10.0))
        .border_width(5.0)
        .border_style(BorderStyle::Solid);
    let border = style.world_paint_transform(10.0, 20.0, 100.0, 50.0);
    assert_eq!(border, Some(rotate_90.around_origin(10.0, 20.0, 0.0, 0.0)));
    style.transforms.transform_box = TransformBox::ContentBox;
    let content = style.world_paint_transform(10.0, 20.0, 100.0, 50.0);
    assert_eq!(
        content,
        Some(rotate_90.around_origin(10.0, 20.0, 15.0, 15.0))
    );
    style.transforms.transform_box = TransformBox::FillBox;
    assert_eq!(
        style.world_paint_transform(10.0, 20.0, 100.0, 50.0),
        content
    );
    style.transforms.transform_box = TransformBox::ViewBox;
    assert_eq!(style.world_paint_transform(10.0, 20.0, 100.0, 50.0), border);
    style.transforms.transform_box = TransformBox::BorderBox;
    assert_eq!(style.world_paint_transform(10.0, 20.0, 100.0, 50.0), border);
    style.transforms.transform_box = TransformBox::ContentBox;
    style.transforms.origin = Some(TransformOrigin {
        x: LengthSpec::Percent(100.0),
        y: LengthSpec::Percent(100.0),
    });
    assert_eq!(style.resolved_transform_origin(100.0, 50.0), [85.0, 35.0]);
    style.padding_left = Some(LengthSpec::Px(20.0));
    style.padding = None;
    style.border_width = None;
    style.border_style = None;
    style.transforms.origin = Some(TransformOrigin {
        x: LengthSpec::Percent(50.0),
        y: LengthSpec::Percent(50.0),
    });
    assert_eq!(style.resolved_transform_origin(100.0, 50.0), [60.0, 25.0]);
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

#[test]
fn perspective_rotate_y_projects_a_trapezoid() {
    let mat = PaintMat4::perspective(800.0)
        .expect("d")
        .then(PaintMat4::rotate_y(30_f32.to_radians()));
    let pivoted = mat.around_origin(0.0, 0.0, 100.0, 40.0);
    let corners = pivoted
        .projected_corners(0.0, 0.0, 200.0, 80.0)
        .expect("corners");
    let left = dist(corners[0], corners[3]);
    let right = dist(corners[1], corners[2]);
    assert!(
        (left - right).abs() > 4.0,
        "rotateY+perspective must not be a parallelogram: left={left} right={right}"
    );
    let top = dist(corners[0], corners[1]);
    let bottom = dist(corners[3], corners[2]);
    assert!(
        (top - 200.0).abs() > 1.0 || (left - 80.0).abs() > 1.0,
        "projected size must change from the layout box"
    );
    assert!(
        (top - bottom).abs() < 1.0,
        "rotateY trapezoid keeps parallel top/bottom, got top={top} bottom={bottom}"
    );
    let (_, persp) = pivoted.planar_homography().expect("homography");
    assert!(
        persp[0].abs() > 1e-4,
        "perspective row g must be nonzero, got {persp:?}"
    );
    assert!(mat.as_affine().is_none(), "must not squash to 2×3");
}

#[test]
fn transform_origin_moves_rotate_y_perspective_pivot() {
    let mat = PaintMat4::perspective(800.0)
        .expect("d")
        .then(PaintMat4::rotate_y(30_f32.to_radians()));
    let center = mat.around_origin(0.0, 0.0, 100.0, 40.0);
    let zero = mat.around_origin(0.0, 0.0, 0.0, 0.0);
    let c0 = center.project_xy(0.0, 0.0).unwrap();
    let z0 = zero.project_xy(0.0, 0.0).unwrap();
    assert!(
        (c0[0] - z0[0]).abs() > 1.0,
        "origin must move the projected corner, center={c0:?} zero={z0:?}"
    );
    let mut style = LayoutStyle::default().transform_3d(mat);
    let via_default = style
        .world_scene_transform(0.0, 0.0, 200.0, 80.0)
        .expect("3d");
    style.transforms.origin = Some(TransformOrigin {
        x: LengthSpec::Px(0.0),
        y: LengthSpec::Px(0.0),
    });
    let via_zero = style
        .world_scene_transform(0.0, 0.0, 200.0, 80.0)
        .expect("3d zero origin");
    assert_ne!(via_default, via_zero);
    assert_eq!(style.world_paint_transform(0.0, 0.0, 200.0, 80.0), None);
}

#[test]
fn rotate_x_matches_rotate3d_x_axis() {
    let angle = 25_f32.to_radians();
    let a = PaintMat4::rotate_x(angle);
    let b = PaintMat4::rotate3d(1.0, 0.0, 0.0, angle).expect("axis");
    for i in 0..16 {
        assert!(
            (a.m[i] - b.m[i]).abs() < 1e-5,
            "m[{i}] rotateX={} rotate3d={}",
            a.m[i],
            b.m[i]
        );
    }
}

#[test]
fn inherit_typography_fills_opentype_fields() {
    let parent = LayoutStyle::with(|s| {
        s.text_ext.font_features = Some(vec![FontFeatureSetting::new(*b"liga", 0)]);
        s.text_ext.font_variation_settings = Some(vec![FontVariationSetting::new(*b"wght", 600.0)]);
        s.text_ext.font_kerning = Some(FontKerningSpec::None);
        s.text_ext.word_break = Some(WordBreakSpec::BreakAll);
        s.text_ext.line_break = Some(LineBreakSpec::Anywhere);
        s.text_ext.letter_spacing = Some(0.5);
    });
    let mut child = LayoutStyle::default();
    child.inherit_typography_from(&parent);
    assert_eq!(child.text_ext.font_features, parent.text_ext.font_features);
    assert_eq!(child.text_ext.font_kerning, Some(FontKerningSpec::None));
    assert_eq!(child.text_ext.word_break, Some(WordBreakSpec::BreakAll));
    assert_eq!(child.text_ext.line_break, Some(LineBreakSpec::Anywhere));
    assert_eq!(child.text_ext.letter_spacing, Some(0.5));
    child.text_ext.word_break = Some(WordBreakSpec::Normal);
    child.inherit_typography_from(&parent);
    assert_eq!(child.text_ext.word_break, Some(WordBreakSpec::Normal));
}

#[test]
fn inherit_typography_preserves_authoring_color_metadata() {
    let parent = LayoutStyle {
        paint_colors: crate::style_model::PaintColorSlots {
            color: Some(crate::style_model::PaintColor::Oklch {
                l: 0.62,
                c: 0.18,
                h: Some(280.0),
                alpha: 1.0,
            }),
            ..Default::default()
        }
        .into(),
        ..LayoutStyle::default()
    };
    let mut child = LayoutStyle::default();
    child.inherit_typography_from(&parent);
    assert_eq!(child.paint_colors.color, parent.paint_colors.color);

    let mut overridden = LayoutStyle::default().color([1.0, 0.0, 0.0, 1.0]);
    overridden.inherit_typography_from(&parent);
    assert_eq!(
        overridden.paint_colors.color,
        Some(crate::style_model::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]))
    );
}

#[test]
fn logical_padding_rebakes_when_direction_becomes_rtl() {
    let mut layout = LayoutStyle::default();
    layout
        .logical_edges
        .padding
        .set_start(Some(LengthSpec::Px(12.0)));
    layout
        .logical_edges
        .padding
        .set_end(Some(LengthSpec::Px(4.0)));
    layout.resolve_logical_box_edges();
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(4.0)));
    layout.set_writing_direction(DirSpec::Rtl);
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(4.0)));
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
    let pad = layout.resolved_padding();
    assert_eq!((pad.left, pad.right), (4.0, 12.0));
}

#[test]
fn bake_logical_edges_vertical_rl_maps_block_start_to_right() {
    let mut layout = LayoutStyle {
        writing_mode: Some(WritingModeSpec::VerticalRl),
        ..Default::default()
    };
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::BlockStart, Some(LengthSpec::Px(8.0)));
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::InlineStart, Some(LengthSpec::Px(3.0)));
    layout.bake_logical_edges();
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(8.0)));
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(3.0)));
    assert!(layout.padding_left.is_none());
    assert!(layout.padding_bottom.is_none());
}

#[test]
fn bake_logical_edges_vertical_lr_maps_block_start_to_left() {
    let mut layout = LayoutStyle {
        writing_mode: Some(WritingModeSpec::VerticalLr),
        ..Default::default()
    };
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::BlockStart, Some(LengthSpec::Px(8.0)));
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::BlockEnd, Some(LengthSpec::Px(2.0)));
    layout.bake_logical_edges();
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(8.0)));
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(2.0)));
}

#[test]
fn a_hand_built_physical_edge_survives_a_logical_one_on_another_edge() {
    // A builder sets the physical top directly, without recording it, and
    // a logical inline-start: only the edge the logical one lands on is
    // derived.
    let mut layout = LayoutStyle {
        padding_top: Some(LengthSpec::Px(5.0)),
        ..Default::default()
    };
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::InlineStart, Some(LengthSpec::Px(3.0)));
    layout.bake_logical_edges();
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(5.0)));
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(3.0)));
    // In RTL the inline-start moves to the right, and the left it no
    // longer lands on is cleared rather than left holding 3px.
    layout.set_writing_direction(DirSpec::Rtl);
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(3.0)));
    assert!(layout.padding_left.is_none());
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(5.0)));
}

#[test]
fn writing_mode_rebakes_logical_padding_from_top_to_right() {
    let mut layout = LayoutStyle::default();
    layout
        .logical_edges
        .padding
        .set(super::LogicalEdge::BlockStart, Some(LengthSpec::Px(8.0)));
    layout.bake_logical_edges();
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(8.0)));
    layout.set_writing_mode(WritingModeSpec::VerticalRl);
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(8.0)));
    assert!(layout.padding_top.is_none());
}

#[test]
fn writing_mode_physical_flex_direction_swaps_row_and_column() {
    let context = |mode| crate::WritingContext::new(mode, DirSpec::Ltr);
    assert_eq!(
        context(WritingModeSpec::VerticalRl).physical_flex_direction(FlexDirection::Row),
        FlexDirection::Column
    );
    assert_eq!(
        context(WritingModeSpec::VerticalLr).physical_flex_direction(FlexDirection::Column),
        FlexDirection::Row
    );
    assert_eq!(
        context(WritingModeSpec::HorizontalTb).physical_flex_direction(FlexDirection::Row),
        FlexDirection::Row
    );
}

#[test]
fn baseline_from_ascent_uses_shaped_metrics_not_fixed_em() {
    let layout = LayoutStyle {
        font_size: Some(20.0),
        padding_top: Some(LengthSpec::Px(2.0)),
        border_width: Some(1.0),
        ..LayoutStyle::default()
    };
    let approx = layout.approximate_baseline(20.0);
    assert!((approx - (2.0 + 1.0 + 16.0)).abs() < 1e-4);
    let shaped = layout.baseline_from_ascent(20.0, Some(9.0));
    assert!((shaped - 12.0).abs() < 1e-4);
    assert!((shaped - approx).abs() > 1.0);
}

#[test]
fn overlay_css_size_overrides_control_tokens() {
    let mut layout = LayoutStyle {
        min_height: Some(LengthSpec::Px(32.0)),
        border_radius: Some(6.0),
        ..LayoutStyle::default()
    };
    let css = LayoutStyle {
        height: Some(LengthSpec::Px(48.0)),
        width: Some(LengthSpec::Px(120.0)),
        min_width: Some(LengthSpec::Px(80.0)),
        border_radius: Some(2.0),
        ..LayoutStyle::default()
    };
    layout.overlay_css_size_overrides(&css);
    assert_eq!(layout.height, Some(LengthSpec::Px(48.0)));
    assert_eq!(layout.width, Some(LengthSpec::Px(120.0)));
    assert_eq!(layout.min_width, Some(LengthSpec::Px(80.0)));
    assert!(layout.min_height.is_none());
    assert_eq!(layout.border_radius, Some(2.0));
}

#[test]
fn inherit_typography_fills_writing_direction() {
    let parent = LayoutStyle {
        dir: Some(DirSpec::Rtl),
        writing_mode: Some(WritingModeSpec::VerticalRl),
        ..LayoutStyle::default()
    };
    let mut child = LayoutStyle::default();
    child.inherit_typography_from(&parent);
    assert_eq!(child.dir, Some(DirSpec::Rtl));
    assert_eq!(child.writing_mode, Some(WritingModeSpec::VerticalRl));
    child.dir = Some(DirSpec::Ltr);
    child.inherit_typography_from(&parent);
    assert_eq!(child.dir, Some(DirSpec::Ltr));
}

#[test]
fn inset_box_shadow_is_distinct_from_outset() {
    let outset = BoxShadowSpec {
        offset_x: 2.0,
        offset_y: 2.0,
        blur_radius: 4.0,
        spread_radius: 0.0,
        color: [0.0, 0.0, 0.0, 1.0].into(),
        inset: false,
    };
    let inset = BoxShadowSpec {
        inset: true,
        ..outset
    };
    assert_ne!(outset, inset);
    assert!(inset.inset);
    assert!(!outset.inset);
}

/// A border whose colour is not on the `LayoutStyle` still strokes, as long
/// as the caller says what colour it will stroke with.
///
/// This is the `Outlined` card: `border_width: Some(1.0)` on the layout, no
/// `border_color` on the layout (the component named a
/// `SemanticColorRole`, which resolves onto the computed style), and a
/// painted width of zero. Width and colour were coming from two different
/// decisions.
#[test]
fn a_border_coloured_outside_the_layout_still_strokes_when_the_caller_names_it() {
    let style = LayoutStyle {
        border_width: Some(1.0),
        ..LayoutStyle::default()
    };
    assert_eq!(style.resolved_border_color(), None);
    assert!(
        style.paint_border_edges().is_zero(),
        "with no colour anywhere there is nothing to stroke"
    );

    let semantic = [0.16, 0.16, 0.16, 1.0];
    let edges = style.paint_border_edges_with(Some(semantic));
    assert_eq!(edges.top, 1.0);
    assert_eq!(edges.left, 1.0);

    // A colour on the layout keeps winning, and the fallback does not
    // resurrect a side the author switched off.
    let none_style = LayoutStyle {
        border_width: Some(1.0),
        border_style: Some(BorderStyle::None),
        ..LayoutStyle::default()
    };
    assert!(
        none_style.paint_border_edges_with(Some(semantic)).is_zero(),
        "`border-style: none` still means no stroke"
    );

    // Zero width stays zero: the fallback supplies a colour, never a width.
    let flat = LayoutStyle {
        border_width: Some(0.0),
        ..LayoutStyle::default()
    };
    assert!(flat.paint_border_edges_with(Some(semantic)).is_zero());
}

#[test]
fn subgrid_requires_grid_display_container() {
    let mut style = LayoutStyle::default().grid_columns_subgrid(true);
    assert!(
        !style.is_subgrid_columns(),
        "subgrid must require display: grid or inline-grid"
    );
    style = style.display(DisplaySpec::Inline);
    assert!(!style.is_subgrid_columns());
    style = style.display(DisplaySpec::Grid);
    assert!(style.is_subgrid_columns());
    style = style.display(DisplaySpec::InlineGrid);
    assert!(style.is_subgrid_columns());
}
