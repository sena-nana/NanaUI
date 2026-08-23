use super::clip::{self, LogicalRect, physical_scissor};
use crate::gpu_texture::{GpuTexturePipeline, GpuTexturePrimitive, HostTextureLayer};
use crate::gpu_view::intersect_physical;
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
        affine: [f32; 6],
        clip: PhysicalRect,
        opacity: f32,
        corner_radius: f32,
        rounded_clip: LogicalRect,
        fragment_clip: super::clip::FragmentClip,
        physical_size: [u32; 2],
        scale_factor: f32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) -> PreparedHostTexture {
        let primitive = GpuTexturePrimitive::from_scene(
            node,
            slot,
            HostTextureLayer::from_binding(binding)
                .with_opacity(opacity)
                .with_corner_radius(corner_radius)
                .with_clip(crate::geometry::LogicalRect::new(
                    rounded_clip.x,
                    rounded_clip.y,
                    rounded_clip.width,
                    rounded_clip.height,
                ))
                .with_fragment_clip(
                    fragment_clip.rect,
                    fragment_clip.inv_abcd,
                    fragment_clip.inv_ef,
                ),
        );
        primitive.prepare(
            &mut self.pipeline,
            device,
            queue,
            crate::geometry::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            affine,
            scale_factor,
            physical_size,
            gpu_work,
        );
        let world = clip::transformed_aabb(bounds, affine);
        let clip = physical_scissor(world, scale_factor, physical_size)
            .map(|world| intersect_physical(world, clip))
            .unwrap_or(PhysicalRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        PreparedHostTexture { primitive, clip }
    }

    pub(super) fn draw(
        &self,
        prepared: &PreparedHostTexture,
        pass: &mut wgpu::RenderPass<'_>,
        dest_size: [u32; 2],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        prepared
            .primitive
            .draw_in_pass(&self.pipeline, pass, prepared.clip, dest_size, gpu_work);
    }

    pub(super) fn trim(&mut self) {
        self.pipeline.trim();
    }
}
