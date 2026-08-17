//! Snapshot-only host texture and `"gpu-view"` Scene painter registration.
//!
//! Uses the snapshot Device/Queue. The `"gpu-view"` painter is the product
//! [`nana_ui::DefaultGpuViewRenderer`]; CPU readback stays in snapshot tooling.

use std::sync::Arc;

use iced::Color;
use iced_wgpu::wgpu;
use nana_ui::runtime::GPU_VIEW_RENDERER;
use nana_ui::{
    DefaultGpuViewRenderer, GpuViewPalette, HostTexture, HostTextureAlphaMode, HostTextureRegistry,
    SceneGpuRendererRegistry,
};

pub const SNAPSHOT_GPU_SLOT: &str = "snapshot-gpu";
const TEXTURE_SIZE: u32 = 32;

pub struct SnapshotGpu {
    _texture: wgpu::Texture,
    pub host_texture: HostTexture,
    pub textures: HostTextureRegistry,
    pub renderers: SceneGpuRendererRegistry,
}

pub fn create_snapshot_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    background: Color,
    accent: Color,
) -> SnapshotGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui snapshot host texture"),
        size: wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let bytes_per_row = TEXTURE_SIZE * 4;
    let aligned_row = bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let pixel = color_rgba8(accent);
    let mut pixels = vec![0_u8; aligned_row as usize * TEXTURE_SIZE as usize];
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let offset = (y * aligned_row + x * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_row),
            rows_per_image: Some(TEXTURE_SIZE),
        },
        wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
    );
    let host_texture = HostTexture::from_wgpu(1, 0, texture.create_view(&Default::default()));
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
        _texture: texture,
        host_texture,
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
