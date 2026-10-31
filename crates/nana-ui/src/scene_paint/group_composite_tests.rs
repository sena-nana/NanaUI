//! 组区域合成回归：覆盖填充成本、嵌套透明度与抗锯齿边缘。

use super::*;

/// 小透明组保留子节点的绘制顺序，同时把合成填充限制在内容范围。
#[test]
fn small_nested_opacity_groups_limit_composite_fill_and_keep_pixels() {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let mut scene = UiScene::new();
    scene.apply_delta(
        [
            translucent_parent(1, &[2, 3], 0.0, 0.0, 200.0, 100.0, 0.5),
            colored_quad_child(2, 1, 10.0, 10.0, 20.0, 20.0, [1.0, 0.0, 0.0, 1.0]),
            child_of(translucent_parent(3, &[4], 0.0, 0.0, 200.0, 100.0, 0.5), 1),
            colored_quad_child(4, 3, 50.0, 10.0, 20.0, 20.0, [0.0, 0.0, 1.0, 1.0]),
        ],
        [],
    );
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [200.0, 100.0],
        [200, 100],
        1.0,
    );
    let counts = painter.last_dest_pass_counts.expect("组帧统计");
    assert!(counts.group >= 2, "嵌套组：{counts:?}");
    assert!(
        counts.composite_pixels > 0 && counts.composite_pixels < 4_000,
        "小组填充：{counts:?}"
    );
    assert!(pixel(&pixels, 200, 20, 20)[0].abs_diff(128) <= 2);
    assert!(pixel(&pixels, 200, 60, 20)[2].abs_diff(64) <= 2);
    assert_eq!(pixel(&pixels, 200, 40, 20), [0, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 200, 190, 90), [0, 0, 0, 255]);
}

/// 非整数边缘的覆盖度须落在保守裁剪范围内，组外像素保持背景。
#[test]
fn normal_group_keeps_antialias_fringe_and_clips_at_target_edge() {
    use nana_ui_runtime::BlendMode;
    let pixels = paint_one_painter(
        PaintFn(|cx: &mut nana_ui_runtime::PaintContext<'_>| {
            cx.fill_path(&rect_path(0.0, 0.0, 80.0, 40.0), [0.0, 1.0, 0.0, 1.0]);
            cx.push_layer(0.5, BlendMode::Normal);
            let mut disc = nana_ui_runtime::PaintPath::new();
            disc.ellipse(LayoutBox {
                x: 10.3,
                y: 10.6,
                width: 19.0,
                height: 19.0,
            });
            cx.fill_path(&disc, [1.0, 0.0, 0.0, 1.0]);
            cx.push_layer(0.5, BlendMode::Normal);
            cx.fill_path(&rect_path(70.0, 10.0, 30.0, 20.0), [0.0, 0.0, 1.0, 1.0]);
            cx.pop_layer();
            cx.pop_layer();
        }),
        80,
        40,
    );
    let at = |x, y| pixel(&pixels, 80, x, y);
    let centre = at(20, 20);
    assert!(
        centre[0].abs_diff(128) <= 2 && centre[1].abs_diff(128) <= 2,
        "{centre:?}"
    );
    let rim = at(10, 20);
    assert!(rim[0] > 0 && rim[0] < centre[0], "AA 边缘：{rim:?}");
    for (x, y) in [(9, 20), (20, 8), (31, 20), (20, 31), (50, 20)] {
        assert_eq!(at(x, y), [0, 255, 0, 255], "组外 ({x},{y})");
    }
    let clipped = at(79, 20);
    assert!(
        clipped[2].abs_diff(64) <= 2 && clipped[1].abs_diff(191) <= 2,
        "屏边：{clipped:?}"
    );
}

/// 阴影扩散出源图形边界时，合成范围继续保守覆盖目标。
#[test]
fn spreading_filter_keeps_pixels_outside_source_bounds() {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let mut scene = UiScene::new();
    scene.apply_delta(
        [extracted_div(
            1,
            &[],
            8.0,
            8.0,
            16.0,
            16.0,
            nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    filter: Some(nana_ui_core::ColorFilter {
                        drop_shadow: Some(nana_ui_core::FilterDropShadow {
                            paint_color: None,
                            offset_x: 16.0,
                            offset_y: 0.0,
                            blur_radius: 0.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
            Some([1.0, 0.0, 0.0, 1.0]),
        )],
        [],
    );
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [64.0, 64.0],
        [64, 64],
        1.0,
    );
    let counts = painter.last_dest_pass_counts.expect("滤镜组统计");
    assert_eq!(counts.composite_pixels, 64 * 64, "扩散滤镜：{counts:?}");
    assert!(pixel(&pixels, 64, 12, 16)[0] > 250);
    assert!(pixel(&pixels, 64, 32, 16)[2] > 250, "源图形外的阴影");
    assert_eq!(pixel(&pixels, 64, 4, 16), [0, 0, 0, 255]);
}
