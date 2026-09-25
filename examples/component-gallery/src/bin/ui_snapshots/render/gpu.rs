//! Snapshot-only host texture and `"gpu-view"` Scene painter registration.
//!
//! Uses the snapshot device. The `"gpu-view"` painter is the product
//! [`nana_ui::DefaultGpuViewRenderer`]; CPU readback stays in snapshot tooling.

use std::sync::Arc;

use nana_ui::Color;
use nana_ui::runtime::GPU_VIEW_RENDERER;
use nana_ui::{
    DefaultGpuViewRenderer, GpuContext, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages, GpuViewPalette, HostTexture, HostTextureAlphaMode, HostTextureRegistry,
    SceneGpuRendererRegistry,
};

pub const SNAPSHOT_GPU_SLOT: &str = "snapshot-gpu";
const TEXTURE_SIZE: u32 = 32;

pub struct SnapshotGpu {
    _host_texture: HostTexture,
    pub textures: HostTextureRegistry,
    pub renderers: SceneGpuRendererRegistry,
}

pub fn create_snapshot_gpu(gpu: &GpuContext, background: Color, accent: Color) -> SnapshotGpu {
    let texture = gpu
        .create_texture(&GpuTextureDescriptor {
            label: Some("nana-ui snapshot host texture"),
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            format: GpuTextureFormat::RGBA8_UNORM_SRGB,
            usage: GpuTextureUsages::COPY_DST | GpuTextureUsages::SAMPLED,
        })
        .expect("snapshot host texture");
    let pixel = color_rgba8(accent);
    let pixels = pixel.repeat((TEXTURE_SIZE * TEXTURE_SIZE) as usize);
    gpu.write_texture(
        &texture,
        GpuTextureRegion::full(TEXTURE_SIZE, TEXTURE_SIZE),
        &pixels,
        TEXTURE_SIZE * 4,
    )
    .expect("snapshot host texture upload");
    let host_texture = HostTexture::new(1, 0, &texture);
    let textures = HostTextureRegistry::new();
    textures.register(
        SNAPSHOT_GPU_SLOT,
        host_texture.clone(),
        TEXTURE_SIZE,
        TEXTURE_SIZE,
        HostTextureAlphaMode::Opaque,
    );
    let mut renderers = SceneGpuRendererRegistry::new();
    renderers.insert(
        GPU_VIEW_RENDERER,
        Arc::new(DefaultGpuViewRenderer::with_palette(GpuViewPalette {
            background: color_array(background),
            accent: color_array(accent),
        })),
    );
    SnapshotGpu {
        _host_texture: host_texture,
        textures,
        renderers,
    }
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn color_rgba8(color: Color) -> [u8; 4] {
    [
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}
