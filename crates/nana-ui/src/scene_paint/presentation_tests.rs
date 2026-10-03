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
    ScenePaintViewport, ScenePresentationColorSpace, ScenePresentationProfile, SceneWgpuPainter,
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
            timeout: None,
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
    let gpu = crate::test_gpu::context();
    let device = nana_gpu::__framework::device(&gpu);
    let queue = nana_gpu::__framework::queue(&gpu);
    let (texture, view) = target(device, profile);
    let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, profile);
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
    assert!(pixel[0] > 45 && pixel[0] < 125, "tone-mapped red={pixel:?}");
    assert!(
        pixel[1] < 8,
        "negative gamut channel must map to zero={pixel:?}"
    );
    assert!(
        pixel[2] > 20 && pixel[2] < 100,
        "tone-mapped blue={pixel:?}"
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
    assert!((pixel[0] - encode(0.8225927)).abs() < 0.03, "red={pixel:?}");
    assert!(
        (pixel[1] - encode(0.0331995)).abs() < 0.03,
        "green={pixel:?}"
    );
    assert!(
        (pixel[2] - encode(0.0170853)).abs() < 0.03,
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
