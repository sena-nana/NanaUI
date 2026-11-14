//! GPU readback coverage for the linear-scRGB presentation boundary.
//!
//! The large painter test module predates presentation profiles and mostly
//! exercises the compatibility constructor.  These tests deliberately build
//! a profile painter, execute its destination blit, and inspect the actual
//! target bytes so a WGSL entry point cannot silently remain untested.

use nana_ui_core::{LengthSpec, PaintColor};
use nana_ui_runtime::{AppContext, DocumentId, LayoutBox, PaintContext, PaintPath, Painter, Stack};
use nana_ui_scene::UiScene;

use super::{
    ScenePaintViewport, ScenePresentationColorSpace, ScenePresentationParameters,
    ScenePresentationProfile, SceneWgpuPainter, linear_sc_rgb_to_bt2020, pq_encode_nits,
    tone_map_headroom_rgb,
};

const WIDTH: u32 = 4;
const HEIGHT: u32 = 4;

#[derive(Clone, Copy)]
struct Fill(PaintColor);

impl Painter for Fill {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let mut path = PaintPath::new();
        path.rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: WIDTH as f32,
            height: HEIGHT as f32,
        });
        cx.fill_path(&path, self.0);
    }

    fn paint_key(&self) -> u64 {
        1
    }
}

fn scene_with_fill(color: PaintColor) -> UiScene {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).expect("test document id");
    let node = context
        .create_component(
            document,
            Stack::column(0.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(WIDTH as f32));
                    layout.height = Some(LengthSpec::Px(HEIGHT as f32));
                })
                .painter(Fill(color)),
        )
        .expect("test painter node");
    context
        .resolve_styles(&[node.stable_id()])
        .expect("test styles");
    context
        .layout_document(
            document,
            nana_ui_runtime::LayoutViewport::new(WIDTH as f32, HEIGHT as f32),
        )
        .expect("test layout");
    let mut scene = UiScene::new();
    scene.apply_delta(context.world().extract_document(document), []);
    scene
}

fn empty_scene() -> UiScene {
    UiScene::new()
}

fn target_format(profile: ScenePresentationProfile) -> wgpu::TextureFormat {
    nana_gpu::__framework::format_to_wgpu(profile.target_format)
}

fn target(
    device: &wgpu::Device,
    profile: ScenePresentationProfile,
) -> (wgpu::Texture, wgpu::TextureView) {
    let format = target_format(profile);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui presentation test target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    bytes_per_pixel: usize,
) -> Vec<u8> {
    let row = WIDTH as usize * bytes_per_pixel;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nana-ui presentation test readback"),
        size: (padded * HEIGHT as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = encoder;
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("presentation readback poll");
    let mapped = slice.get_mapped_range().expect("presentation readback map");
    let mut bytes = Vec::with_capacity(row * HEIGHT as usize);
    for line in mapped.chunks_exact(padded) {
        bytes.extend_from_slice(&line[..row]);
    }
    bytes
}

fn paint(
    profile: ScenePresentationProfile,
    scene: UiScene,
    clear_color: [f32; 4],
) -> (Vec<u8>, super::DestPassCounts) {
    paint_with_parameters(
        profile,
        scene,
        clear_color,
        ScenePresentationParameters::default(),
    )
}

fn paint_with_parameters(
    profile: ScenePresentationProfile,
    scene: UiScene,
    clear_color: [f32; 4],
    parameters: ScenePresentationParameters,
) -> (Vec<u8>, super::DestPassCounts) {
    let gpu = crate::test_gpu::context();
    let device = nana_gpu::__framework::device(&gpu);
    let queue = nana_gpu::__framework::queue(&gpu);
    let (texture, view) = target(device, profile);
    let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, profile);
    painter.set_presentation_parameters(parameters);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui presentation test encode"),
    });
    painter
        .paint_encoder(
            &scene,
            &mut encoder,
            &view,
            ScenePaintViewport {
                logical_size: [WIDTH as f32, HEIGHT as f32],
                physical_size: [WIDTH, HEIGHT],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color,
                clear: true,
            },
            None,
            None,
        )
        .expect("profile painter encode");
    let counts = painter
        .last_dest_pass_counts
        .expect("profile paint records pass counts");
    let bytes_per_pixel = match target_format(profile) {
        wgpu::TextureFormat::Rgba16Float => 8,
        _ => 4,
    };
    (
        readback(device, queue, encoder, &texture, bytes_per_pixel),
        counts,
    )
}

fn encode_paint(
    painter: &mut SceneWgpuPainter,
    scene: &UiScene,
    view: &wgpu::TextureView,
    parameters: ScenePresentationParameters,
) -> (wgpu::CommandEncoder, super::DestPassCounts) {
    let gpu = crate::test_gpu::context();
    let device = nana_gpu::__framework::device(&gpu);
    painter.set_presentation_parameters(parameters);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui presentation parameter update"),
    });
    painter
        .paint_encoder(
            scene,
            &mut encoder,
            view,
            ScenePaintViewport {
                logical_size: [WIDTH as f32, HEIGHT as f32],
                physical_size: [WIDTH, HEIGHT],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color: [0.0; 4],
                clear: true,
            },
            None,
            None,
        )
        .expect("parameter update paint");
    (
        encoder,
        painter
            .last_dest_pass_counts
            .expect("parameter update records pass counts"),
    )
}

fn rgba8(bytes: &[u8]) -> [u8; 4] {
    bytes[..4].try_into().expect("rgba8 pixel")
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 => sign * (fraction as f32 / 1024.0) * 2f32.powi(-14),
        0x1f => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        exponent => sign * (1.0 + fraction as f32 / 1024.0) * 2f32.powi(exponent as i32 - 15),
    }
}

fn rgba16(bytes: &[u8]) -> [f32; 4] {
    std::array::from_fn(|index| {
        let offset = index * 2;
        half(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
    })
}

#[test]
fn display_p3_uses_fp16_working_target_and_one_final_transform() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA8_UNORM,
        ScenePresentationColorSpace::DisplayP3,
    );
    assert_eq!(
        profile.working_format(),
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [1.0, 0.0, 0.0],
            alpha: 1.0,
        }),
        [0.0; 4],
    );
    let pixel = rgba8(&pixels);
    // scRGB red -> Display-P3 linear matrix -> one sRGB OETF.  The expected
    // channels leave room for adapter quantisation while rejecting raw sRGB
    // red and an accidental second transfer.
    assert!(pixel[0] > 220 && pixel[0] < 245, "P3 red={pixel:?}");
    assert!(pixel[1] > 35 && pixel[1] < 75, "P3 green={pixel:?}");
    assert!(pixel[2] > 20 && pixel[2] < 60, "P3 blue={pixel:?}");
    assert_eq!(
        counts.color + counts.msaa,
        1,
        "profile scene stays one working color pass"
    );
    assert_eq!(counts.blit, 1, "profile has one final presentation pass");
}

#[test]
fn display_p3_typed_srgb_target_uses_hardware_transfer_once() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB,
        ScenePresentationColorSpace::DisplayP3,
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [1.0, 0.0, 0.0],
            alpha: 1.0,
        }),
        [0.0; 4],
    );
    let pixel = rgba8(&pixels);
    assert!(pixel[0] > 220 && pixel[0] < 245, "P3 red={pixel:?}");
    assert!(pixel[1] > 35 && pixel[1] < 75, "P3 green={pixel:?}");
    assert!(pixel[2] > 20 && pixel[2] < 60, "P3 blue={pixel:?}");
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn extended_linear_profile_keeps_negative_and_overwhite_values() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgbLinear,
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [1.25, -0.125, 0.5],
            alpha: 0.5,
        }),
        [0.0; 4],
    );
    let pixel = rgba16(&pixels);
    assert!(
        (pixel[0] - 0.625).abs() < 0.02,
        "overwhite premult={pixel:?}"
    );
    assert!(
        (pixel[1] + 0.0625).abs() < 0.02,
        "negative premult={pixel:?}"
    );
    assert!((pixel[2] - 0.25).abs() < 0.02, "blue premult={pixel:?}");
    assert!((pixel[3] - 0.5).abs() < 0.02, "alpha={pixel:?}");
    assert!(
        pixel.iter().all(|value| value.is_finite()),
        "fp16 NaN={pixel:?}"
    );
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn extended_linear_clear_uses_the_same_headroom_as_blit() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgbLinear,
    );
    let parameters = ScenePresentationParameters::new(1.0, 80.0);
    let (pixels, counts) =
        paint_with_parameters(profile, empty_scene(), [2.0, 0.5, 0.0, 0.5], parameters);
    let pixel = rgba16(&pixels);
    // H=1 has no room above white: the colour scales as a whole to
    // [1, 0.25, 0] (its hue kept) before premultiplication; ExtendedLinear
    // still stores that result linearly (it does not apply sRGB OETF).
    assert!((pixel[0] - 0.5).abs() < 0.02, "clear={pixel:?}");
    assert!((pixel[1] - 0.125).abs() < 0.02, "clear={pixel:?}");
    assert!(pixel[2].abs() < 0.02, "clear={pixel:?}");
    assert!((pixel[3] - 0.5).abs() < 0.02, "clear alpha={pixel:?}");
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

/// Windows puts scRGB 1.0 at 80 nits: with SDR white at 240 nits, white is
/// written as 3.0, by the clear and by the blit alike.
#[test]
fn extended_linear_white_follows_the_sdr_white_level() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgbLinear,
    );
    let parameters = ScenePresentationParameters::new(4.0, 240.0).with_extended_linear_white(3.0);
    let (pixels, _) =
        paint_with_parameters(profile, empty_scene(), [1.0, 1.0, 1.0, 1.0], parameters);
    let clear = rgba16(&pixels);
    assert!(
        clear[..3].iter().all(|v| (v - 3.0).abs() < 0.02),
        "clear={clear:?}"
    );
    let (pixels, _) = paint_with_parameters(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [1.0; 3],
            alpha: 1.0,
        }),
        [0.0; 4],
        parameters,
    );
    let fill = rgba16(&pixels);
    assert!(
        fill[..3].iter().all(|v| (v - 3.0).abs() < 0.02),
        "fill={fill:?}"
    );
}

#[test]
fn zero_alpha_extended_color_fails_closed_at_presentation_boundary() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgbLinear,
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [2.0, -0.5, f32::NAN],
            alpha: 0.0,
        }),
        [0.0; 4],
    );
    let pixel = rgba16(&pixels);
    assert!(
        pixel.iter().all(|value| value.is_finite()),
        "pixel={pixel:?}"
    );
    assert!(
        pixel.iter().all(|value| value.abs() < 0.01),
        "pixel={pixel:?}"
    );
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

/// An HDR request on an SDR surface leaves everything up to white as it is:
/// white stays white, a light grey stays that grey.
#[test]
fn hdr_to_sdr_fallback_keeps_white_and_light_greys() {
    let profile = ScenePresentationProfile::sdr(nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB)
        .with_force_float_working();
    for (linear, expected) in [(1.0_f32, 255_u8), (0.913, 245)] {
        let (pixels, _) = paint(
            profile,
            scene_with_fill(PaintColor::LinearScRgb {
                channels: [linear; 3],
                alpha: 1.0,
            }),
            [0.0; 4],
        );
        let pixel = rgba8(&pixels);
        for channel in &pixel[..3] {
            assert!(
                channel.abs_diff(expected) <= 1,
                "{linear} became {pixel:?}, not {expected}"
            );
        }
    }
}

#[test]
fn hdr_to_sdr_fallback_tone_maps_highlights_and_preserves_alpha() {
    let profile = ScenePresentationProfile::sdr(nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB)
        .with_force_float_working();
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [2.0, -0.2, 0.5],
            alpha: 0.5,
        }),
        [0.0; 4],
    );
    let pixel = rgba8(&pixels);
    // Scaled as a whole to white, then pulled into gamut toward its grey,
    // then premultiplied and encoded by the typed target.
    let mapped = super::presentation_color::gamut_map_rgb(
        super::presentation_color::tone_map_headroom_rgb([2.0, -0.2, 0.5], 1.0),
    );
    let encode = |value: f32| {
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    for (channel, expected) in pixel[..3].iter().zip(mapped.map(|v| encode(v * 0.5))) {
        assert!(channel.abs_diff(expected) <= 3, "{pixel:?} vs {mapped:?}");
    }
    assert!(
        pixel[1] < 8,
        "negative gamut channel must map to zero={pixel:?}"
    );
    assert!(
        (i16::from(pixel[3]) - 128).unsigned_abs() <= 2,
        "alpha={pixel:?}"
    );
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn extended_srgb_hdr_profile_keeps_encoded_highlights_in_fp16() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgb,
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [2.0, -0.2, 0.5],
            alpha: 0.5,
        }),
        [0.0; 4],
    );
    let pixel = rgba16(&pixels);
    let encode = |value: f32| {
        let value = value.abs();
        if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    };
    // The default Linear alpha encoding stores `encode(linear * alpha)` in
    // this untyped fp16 target, so the over-white straight value is first
    // premultiplied (2.0 * 0.5 = 1.0).
    assert!((pixel[0] - encode(2.0 * 0.5)).abs() < 0.03, "red={pixel:?}");
    assert!(
        (pixel[1] + encode(0.2 * 0.5)).abs() < 0.03,
        "green={pixel:?}"
    );
    assert!(
        (pixel[2] - encode(0.5 * 0.5)).abs() < 0.03,
        "blue={pixel:?}"
    );
    assert!((pixel[3] - 0.5).abs() < 0.02, "alpha={pixel:?}");
    assert!(pixel.iter().all(|value| value.is_finite()));
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn extended_display_p3_hdr_profile_uses_p3_primaries() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedDisplayP3,
    );
    let (pixels, counts) = paint(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [1.0, 0.0, 0.0],
            alpha: 1.0,
        }),
        [0.0; 4],
    );
    let pixel = rgba16(&pixels);
    let encode = |value: f32| {
        if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    };
    assert!((pixel[0] - encode(0.8224621)).abs() < 0.03, "red={pixel:?}");
    assert!(
        (pixel[1] - encode(0.0331941)).abs() < 0.03,
        "green={pixel:?}"
    );
    assert!(
        (pixel[2] - encode(0.0170827)).abs() < 0.03,
        "blue={pixel:?}"
    );
    assert!((pixel[3] - 1.0).abs() < 0.02, "alpha={pixel:?}");
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn presentation_clear_with_transparent_alpha_is_finite() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA8_UNORM,
        ScenePresentationColorSpace::DisplayP3,
    );
    let (pixels, counts) = paint(profile, empty_scene(), [0.2, 0.4, 0.8, 0.25]);
    let pixel = rgba8(&pixels);
    assert!(
        (i16::from(pixel[3]) - 64).unsigned_abs() <= 2,
        "clear alpha={pixel:?}"
    );
    assert!(
        pixel[..3].iter().all(|channel| *channel > 0),
        "clear rgb={pixel:?}"
    );
    assert!(
        pixel.iter().all(|channel| *channel != 0xff),
        "clear overflow={pixel:?}"
    );
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn pq_presentation_matches_reference_encoding_and_preserves_alpha() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::Bt2100Pq,
    );
    // A headroom of one is the SDR reference budget: values at or below
    // reference white pass through unchanged, while highlights are clipped
    // to that same peak before the requested HDR transfer.
    let parameters = ScenePresentationParameters::new(1.0, 80.0);
    let (pixels, counts) = paint_with_parameters(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [0.18, 0.18, 0.18],
            alpha: 0.5,
        }),
        [0.0; 4],
        parameters,
    );
    let pixel = rgba16(&pixels);
    let expected = pq_encode_nits(0.18 * parameters.reference_white_nits()) * 0.5;
    for (actual, expected) in pixel[..3].iter().zip([expected; 3]) {
        assert!(
            (actual - expected).abs() < 0.02,
            "PQ={pixel:?}, expected={expected}"
        );
    }
    assert!((pixel[3] - 0.5).abs() < 0.02, "PQ alpha={pixel:?}");
    assert_eq!(counts.color + counts.msaa, 1);
    assert_eq!(counts.blit, 1);
}

#[test]
fn hlg_presentation_matches_reference_encoding_and_clear() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::Bt2100Hlg,
    );
    let parameters = ScenePresentationParameters::new(4.0, 80.0);
    let rgb = linear_sc_rgb_to_bt2020([0.18, 0.18, 0.18]);
    let expected = super::presentation_color::hlg_encode_display(
        rgb.map(|value| value * parameters.reference_white_nits() / 1_000.0),
    )[0];
    let (pixels, counts) = paint_with_parameters(
        profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [0.18, 0.18, 0.18],
            alpha: 1.0,
        }),
        [0.0; 4],
        parameters,
    );
    let pixel = rgba16(&pixels);
    for actual in &pixel[..3] {
        assert!(
            (actual - expected).abs() < 0.02,
            "HLG={pixel:?}, expected={expected}"
        );
    }
    assert!((pixel[3] - 1.0).abs() < 0.02);
    assert_eq!(counts.blit, 1);

    // The empty-scene clear uses the same presentation branch as a drawn
    // pixel, including transfer, alpha and BT.2020 primaries.
    let (cleared, _) =
        paint_with_parameters(profile, empty_scene(), [0.18, 0.18, 0.18, 0.5], parameters);
    let clear_pixel = rgba16(&cleared);
    for actual in &clear_pixel[..3] {
        assert!(
            (actual - expected * 0.5).abs() < 0.02,
            "HLG clear={clear_pixel:?}"
        );
    }
    assert!(
        (clear_pixel[3] - 0.5).abs() < 0.02,
        "HLG clear alpha={clear_pixel:?}"
    );
}

#[test]
fn headroom_update_reuses_scene_and_changes_only_presentation_blit() {
    let profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::Bt2100Pq,
    );
    let gpu = crate::test_gpu::context();
    let device = nana_gpu::__framework::device(&gpu);
    let queue = nana_gpu::__framework::queue(&gpu);
    let (texture, view) = target(device, profile);
    let scene = scene_with_fill(PaintColor::LinearScRgb {
        channels: [2.0, 1.0, 0.5],
        alpha: 1.0,
    });
    let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, profile);
    let (first_encoder, first_counts) = encode_paint(
        &mut painter,
        &scene,
        &view,
        ScenePresentationParameters::new(1.0, 80.0),
    );
    let first = readback(device, queue, first_encoder, &texture, 8);
    let (second_encoder, second_counts) = encode_paint(
        &mut painter,
        &scene,
        &view,
        ScenePresentationParameters::new(16.0, 80.0),
    );
    let second = readback(device, queue, second_encoder, &texture, 8);
    let first_pixel = rgba16(&first);
    let second_pixel = rgba16(&second);
    let first_expected = linear_sc_rgb_to_bt2020(tone_map_headroom_rgb([2.0, 1.0, 0.5], 1.0))
        .map(|channel| pq_encode_nits(channel * 80.0));
    for (actual, expected) in first_pixel[..3].iter().zip(first_expected) {
        assert!(
            (actual - expected).abs() < 0.02,
            "headroom=1 highlight={first_pixel:?}, expected={first_expected:?}"
        );
    }
    assert!(
        (first_pixel[0] - second_pixel[0]).abs() > 0.01,
        "headroom update did not change highlight: {first_pixel:?} -> {second_pixel:?}"
    );
    // The shared shoulder keeps equal scene channels neutral. The saturated
    // sample above may change chroma when only one channel is above white,
    // but the midtone channels themselves are not globally rescaled.
    let neutral_scene = scene_with_fill(PaintColor::LinearScRgb {
        channels: [2.0, 2.0, 2.0],
        alpha: 1.0,
    });
    let (neutral_first_encoder, _) = encode_paint(
        &mut painter,
        &neutral_scene,
        &view,
        ScenePresentationParameters::new(1.0, 80.0),
    );
    let neutral_first = rgba16(&readback(device, queue, neutral_first_encoder, &texture, 8));
    let (neutral_second_encoder, _) = encode_paint(
        &mut painter,
        &neutral_scene,
        &view,
        ScenePresentationParameters::new(16.0, 80.0),
    );
    let neutral_second = rgba16(&readback(
        device,
        queue,
        neutral_second_encoder,
        &texture,
        8,
    ));
    for pixel in [neutral_first, neutral_second] {
        assert!(
            (pixel[1] / pixel[0] - 1.0).abs() < 0.02,
            "neutral={pixel:?}"
        );
        assert!(
            (pixel[2] / pixel[0] - 1.0).abs() < 0.02,
            "neutral={pixel:?}"
        );
    }
    assert_eq!(first_counts.color + first_counts.msaa, 1);
    assert_eq!(first_counts.blit, 1);
    assert_eq!(second_counts.color + second_counts.msaa, 0);
    assert_eq!(second_counts.blit, 1);
    assert_eq!(painter.presentation_parameters().headroom(), 16.0);
}

#[test]
fn plain_sdr_profile_keeps_srgb_fast_path_and_auto_promotes_extended_scene_and_host_textures() {
    let profile = ScenePresentationProfile::sdr(nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB);
    let gpu = crate::test_gpu::context();
    let device = nana_gpu::__framework::device(&gpu);
    let queue = nana_gpu::__framework::queue(&gpu);
    let (texture, view) = target(device, profile);
    let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, profile);

    // 1. Ordinary sRGB scene keeps the zero-extra-VRAM 8-bit SDR working target (4 B/px, no fp16 pipeline set).
    let srgb_scene = scene_with_fill(PaintColor::srgb([0.2, 0.4, 0.8, 1.0]));
    let (srgb_encoder, srgb_counts) = encode_paint(
        &mut painter,
        &srgb_scene,
        &view,
        ScenePresentationParameters::default(),
    );
    let _ = readback(device, queue, srgb_encoder, &texture, 4);
    assert_eq!(
        painter.working_format(),
        nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB
    );
    assert!(
        painter.pipeline_sets.is_empty(),
        "pure sRGB scenes must not allocate secondary float pipeline sets"
    );
    assert_eq!(srgb_counts.blit, 1);

    // 2. Over-white / wide-gamut scene automatically promotes to RGBA16_FLOAT working target
    // and applies controlled shoulder + gamut compression instead of [255, 255, 0] hardware clipping.
    let extended_scene = scene_with_fill(PaintColor::LinearScRgb {
        channels: [2.0, -0.2, 0.5],
        alpha: 1.0,
    });
    let (ext_encoder, ext_counts) = encode_paint(
        &mut painter,
        &extended_scene,
        &view,
        ScenePresentationParameters::default(),
    );
    let ext_bytes = readback(device, queue, ext_encoder, &texture, 4);
    let ext_pixel = rgba8(&ext_bytes);
    assert_eq!(
        painter.working_format(),
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT
    );
    assert_eq!(ext_counts.blit, 1);

    let mapped = super::presentation_color::gamut_map_rgb(
        super::presentation_color::tone_map_headroom_rgb([2.0, -0.2, 0.5], 1.0),
    );
    let encode_srgb = |value: f32| {
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    for (actual, expected) in ext_pixel[..3].iter().zip(mapped.map(encode_srgb)) {
        assert!(
            actual.abs_diff(expected) <= 3,
            "auto-promoted SDR pixel={ext_pixel:?}, expected={mapped:?}"
        );
    }

    // 3. Returning to an ordinary sRGB scene drops back to the 8-bit SDR working target.
    let (back_encoder, back_counts) = encode_paint(
        &mut painter,
        &srgb_scene,
        &view,
        ScenePresentationParameters::default(),
    );
    let _ = readback(device, queue, back_encoder, &texture, 4);
    assert_eq!(
        painter.working_format(),
        nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB
    );
    assert_eq!(back_counts.blit, 1);

    // 4. An RGBA16_FLOAT HostTexture on an otherwise empty/sRGB scene also promotes to float working.
    let mut context = AppContext::new();
    let document = DocumentId::new(1).expect("doc");
    let preview = context
        .create_component(document, nana_ui_runtime::GpuTextureView::new("hdr_tex"))
        .expect("gpu texture view");
    let mut layout = nana_ui_runtime::MutationQueue::new();
    layout.write_layout(
        preview.stable_id(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: WIDTH as f32,
            height: HEIGHT as f32,
        },
    );
    context.commit_mutations(layout).expect("commit layout");
    let work = context.take_system_work();
    context.resolve_styles(&work.style).expect("resolve styles");
    let mut tex_scene = UiScene::new();
    tex_scene.apply_delta(
        context.world().extract_nodes(&work.render_extraction),
        work.render_removals,
    );
    let fp16_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr host texture"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let fp16_view = fp16_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let mut init_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hdr host texture clear"),
    });
    {
        let _pass = init_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear hdr tex"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &fp16_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 2.0,
                        g: 0.5,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    let registry = crate::HostTextureRegistry::new();
    registry.register(
        "hdr_tex",
        crate::HostTexture::new(&crate::test_gpu::wrap_view(&fp16_view)),
        WIDTH,
        HEIGHT,
        crate::HostTextureAlphaMode::Premultiplied,
    );
    painter
        .paint_encoder(
            &tex_scene,
            &mut init_encoder,
            &view,
            ScenePaintViewport {
                logical_size: [WIDTH as f32, HEIGHT as f32],
                physical_size: [WIDTH, HEIGHT],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color: [0.0; 4],
                clear: true,
            },
            Some(&registry),
            None,
        )
        .expect("paint hdr host texture");
    let tex_counts = painter.last_dest_pass_counts.expect("pass counts");
    let tex_bytes = readback(device, queue, init_encoder, &texture, 4);
    let tex_pixel = rgba8(&tex_bytes);
    assert_eq!(
        painter.working_format(),
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT
    );
    assert_eq!(tex_counts.blit, 1);
    assert!(
        tex_pixel[1] < 220,
        "tone-mapped [2.0, 0.5, 0.0] highlight keeps orange ratio rather than clipping green high: {tex_pixel:?}"
    );
}

#[test]
fn display_p3_sdr_tone_maps_over_white_highlights_and_soft_light_handles_negative_backdrop() {
    let p3_profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA8_UNORM_SRGB,
        ScenePresentationColorSpace::DisplayP3,
    );
    let (p3_bytes, p3_counts) = paint(
        p3_profile,
        scene_with_fill(PaintColor::LinearScRgb {
            channels: [2.0, 0.5, 0.0],
            alpha: 1.0,
        }),
        [0.0; 4],
    );
    let p3_pixel = rgba8(&p3_bytes);
    assert_eq!(p3_counts.blit, 1);
    // Without tone mapping `[2.0, 0.5, 0.0]` would clip red at 1.0 while leaving green disproportionately high.
    let expected_p3 =
        super::presentation_color::gamut_map_rgb(super::presentation_color::linear_srgb_to_p3(
            super::presentation_color::tone_map_headroom_rgb([2.0, 0.5, 0.0], 1.0),
        ));
    let encode_srgb = |value: f32| {
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    for (actual, expected) in p3_pixel[..3].iter().zip(expected_p3.map(encode_srgb)) {
        assert!(
            actual.abs_diff(expected) <= 3,
            "Display-P3 SDR tone-mapped pixel={p3_pixel:?}, expected={expected_p3:?}"
        );
    }

    // Soft-light blend over a negative wide-gamut backdrop must remain finite (no sqrt(<0) NaN).
    let mut context = AppContext::new();
    let document = DocumentId::new(1).expect("doc");
    let root = context
        .create_component(
            document,
            Stack::column(0.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(WIDTH as f32));
                    layout.height = Some(LengthSpec::Px(HEIGHT as f32));
                })
                .painter(Fill(PaintColor::LinearScRgb {
                    channels: [-0.25, 0.8, -0.1],
                    alpha: 1.0,
                })),
        )
        .expect("backdrop");
    let overlay = context
        .create_component(
            document,
            Stack::column(0.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(WIDTH as f32));
                    layout.height = Some(LengthSpec::Px(HEIGHT as f32));
                    layout.paint.mix_blend = nana_ui_core::MixBlendMode::SoftLight;
                })
                .painter(Fill(PaintColor::srgb([0.8, 0.7, 0.6, 1.0]))),
        )
        .expect("overlay");
    context.append_child(root, overlay).expect("append");
    context
        .resolve_styles(&[root.stable_id(), overlay.stable_id()])
        .expect("styles");
    context
        .layout_document(
            document,
            nana_ui_runtime::LayoutViewport::new(WIDTH as f32, HEIGHT as f32),
        )
        .expect("layout");
    let mut blend_scene = UiScene::new();
    blend_scene.apply_delta(context.world().extract_document(document), []);

    let hdr_profile = ScenePresentationProfile::new(
        nana_gpu::GpuTextureFormat::RGBA16_FLOAT,
        ScenePresentationColorSpace::ExtendedSrgbLinear,
    );
    let (blend_bytes, _) = paint(hdr_profile, blend_scene, [0.0; 4]);
    let blend_pixel = rgba16(&blend_bytes);
    assert!(
        blend_pixel.iter().all(|channel| channel.is_finite()),
        "soft-light over negative wide-gamut backdrop produced non-finite pixel: {blend_pixel:?}"
    );
}
