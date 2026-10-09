//! Superellipse corners: the scene's [`CornerShape`] shapes a quad's own
//! corners and the rounded clips around its children, on the quad path and
//! on the HostTexture path. Round stays the circular arc it always was, and
//! two scenes painted by one painter keep their own shapes.
use super::*;
use nana_ui_core::CornerShape;

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// A green 64px square at the origin, every corner a 24px radius.
fn rounded_square(shape: Option<CornerShape>) -> UiScene {
    let mut scene = UiScene::new();
    let style = nana_ui_core::LayoutStyle {
        background: Some(GREEN),
        border_radius: Some(24.0),
        ..Default::default()
    };
    scene.apply_delta(
        [extracted_div(
            1,
            &[],
            0.0,
            0.0,
            64.0,
            64.0,
            style,
            Some(GREEN),
        )],
        [],
    );
    if let Some(shape) = shape {
        scene.set_corner_shape(shape);
    }
    scene
}

fn paint_square(painter: &mut SceneWgpuPainter, scene: &UiScene) -> Vec<u8> {
    let (device, queue) = test_device();
    paint_scene_rgba(&device, &queue, painter, scene, [64.0, 64.0], [64, 64], 1.0)
}

/// The green a box from the origin with corner radius `r` and corner
/// exponent `n` paints at pixel `(px, py)` by its top-left corner: the
/// curve's distance to first order along its normal, ramped over one pixel.
/// `None` off the corner and more than a pixel and a half from its curve.
fn corner_green(px: u32, py: u32, r: f32, n: f32) -> Option<f32> {
    let q = [r - (px as f32 + 0.5), r - (py as f32 + 0.5)];
    if q[0] <= 0.0 || q[1] <= 0.0 {
        return None;
    }
    let norm = (q[0].powf(n) + q[1].powf(n)).powf(1.0 / n);
    let gradient = [(q[0] / norm).powf(n - 1.0), (q[1] / norm).powf(n - 1.0)];
    let distance = (norm - r) / gradient[0].hypot(gradient[1]);
    (distance.abs() <= 1.5).then(|| (0.5 - distance).clamp(0.0, 1.0) * 255.0)
}

#[test]
fn a_squircle_scene_paints_superellipse_corners() {
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let default = paint_square(&mut painter, &rounded_square(None));
    let round = paint_square(&mut painter, &rounded_square(Some(CornerShape::Round)));
    let squircle = paint_square(&mut painter, &rounded_square(Some(CornerShape::SQUIRCLE)));
    assert_eq!(default, round, "a scene's corners are round by default");

    let exponent = CornerShape::SQUIRCLE.exponent();
    let mut checked = 0;
    for py in 0..24 {
        for px in 0..24 {
            let Some(expected) = corner_green(px, py, 24.0, exponent) else {
                continue;
            };
            let green = f32::from(pixel(&squircle, 64, px, py)[1]);
            assert!(
                (green - expected).abs() <= 16.0,
                "({px},{py}): expected green {expected:.0} on the superellipse, got {green}"
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked} pixels near the curve");

    // On the diagonal the squircle reaches 3.8px from the corner, the arc
    // only 7px: (5, 5) is well inside one and well outside the other.
    assert_eq!(pixel(&squircle, 64, 5, 5)[1], 255);
    assert_eq!(pixel(&round, 64, 5, 5)[1], 0);
    // The straight sides do not move.
    for (px, py) in [(32, 0), (0, 32), (32, 63), (63, 32), (32, 32)] {
        assert_eq!(pixel(&squircle, 64, px, py), pixel(&round, 64, px, py));
    }
}

#[test]
fn a_squircle_clip_keeps_what_a_round_one_cuts() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut painter = SceneWgpuPainter::for_test(format);
    let view = solid_texture_view(&device, &queue, format, 64, 64, wgpu::Color::GREEN);
    let registry = register_host_texture("layer", &view, 64, 64);
    for shape in [CornerShape::Round, CornerShape::SQUIRCLE] {
        // (6, 6) is 4px outside the parent's 32px arc and 2px inside its
        // superellipse.
        let wanted = if shape == CornerShape::Round { 0 } else { 255 };

        let mut quad = UiScene::new();
        quad.apply_delta(
            rounded_clip_around(colored_quad_child(2, 1, 0.0, 0.0, 64.0, 64.0, GREEN)),
            [],
        );
        quad.set_corner_shape(shape);
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &quad,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        assert_eq!(
            pixel(&pixels, 64, 6, 6)[1],
            wanted,
            "a quad under {shape:?}"
        );
        assert_eq!(pixel(&pixels, 64, 1, 1)[1], 0, "a quad under {shape:?}");

        let mut texture = UiScene::new();
        texture.apply_delta(
            rounded_clip_around(host_texture_child(2, 1, 0.0, 0.0, 64.0, 64.0, "layer")),
            [],
        );
        texture.set_corner_shape(shape);
        let (target, target_view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui host texture under a shaped parent clip"),
        });
        painter
            .paint_encoder(
                &texture,
                &mut encoder,
                &target_view,
                ScenePaintViewport {
                    logical_size: [64.0, 64.0],
                    physical_size: [64, 64],
                    scale_factor: 1.0,
                    scene_origin: [0.0, 0.0],
                    target_origin: [0.0, 0.0],
                    clear_color: [0.0, 0.0, 0.0, 1.0],
                    clear: true,
                },
                Some(&registry),
                None,
            )
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &target, 64, 64);
        assert_eq!(
            pixel(&pixels, 64, 6, 6)[1],
            wanted,
            "a host texture under {shape:?}"
        );
        assert_eq!(
            pixel(&pixels, 64, 1, 1)[1],
            0,
            "a host texture under {shape:?}"
        );
    }
    drop(view);
}

/// Two documents (two windows, or an offscreen session beside one) share a
/// painter: each frame paints its own scene's shape, whatever came before.
#[test]
fn one_painter_paints_each_scenes_own_corner_shape() {
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let round = rounded_square(Some(CornerShape::Round));
    let squircle = rounded_square(Some(CornerShape::SQUIRCLE));
    for (scene, wanted) in [(&round, 0), (&squircle, 255), (&round, 0), (&squircle, 255)] {
        let pixels = paint_square(&mut painter, scene);
        assert_eq!(pixel(&pixels, 64, 5, 5)[1], wanted);
    }
}
