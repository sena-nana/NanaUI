use super::clip::LogicalRect;
use crate::gpu_texture::{GpuTexturePipeline, GpuTexturePrimitive, HostTextureLayer};
use crate::{HostTextureBinding, PhysicalRect};

pub(super) struct HostTexturePipeline {
    pipeline: GpuTexturePipeline,
}

pub(super) struct PreparedHostTexture {
    primitive: GpuTexturePrimitive,
    clip: PhysicalRect,
}

impl HostTexturePipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            pipeline: GpuTexturePipeline::new(device, queue, format),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: HostTextureBinding,
        node: u64,
        slot: u8,
        bounds: LogicalRect,
        clip: PhysicalRect,
        opacity: f32,
        _physical_size: [u32; 2],
        scale_factor: f32,
    ) -> PreparedHostTexture {
        let primitive = GpuTexturePrimitive::from_scene(
            node,
            slot,
            HostTextureLayer::from_binding(binding).with_opacity(opacity),
        );
        primitive.prepare(
            &mut self.pipeline,
            device,
            queue,
            crate::geometry::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            scale_factor,
        );
        PreparedHostTexture { primitive, clip }
    }

    pub(super) fn render(
        &self,
        prepared: &PreparedHostTexture,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        prepared
            .primitive
            .render(&self.pipeline, encoder, target, prepared.clip);
    }

    pub(super) fn trim(&mut self) {
        self.pipeline.trim();
    }
}
