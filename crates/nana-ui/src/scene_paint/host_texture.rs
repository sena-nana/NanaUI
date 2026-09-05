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
    pub(super) fn set_image_waker(&mut self, wake: super::url_texture_cache::ImageWake) {
        self.pipeline.set_image_waker(wake);
    }
    pub(super) fn has_image_updates(&self) -> bool {
        self.pipeline.has_image_updates()
    }
    pub(super) fn has_pending_images(&self) -> bool {
        self.pipeline.has_pending_images()
    }
    pub(super) fn begin_frame(&mut self) {
        self.pipeline.begin_frame();
    }
    pub(super) fn poll_images(&mut self) -> bool {
        self.pipeline.poll_images()
    }
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
        slot: u64,
        bounds: LogicalRect,
        affine: [f32; 6],
        persp: [f32; 2],
        clip: PhysicalRect,
        opacity: f32,
        corner_radius: f32,
        rounded_clip: LogicalRect,
        fragment_clip: super::clip::FragmentClip,
        physical_size: [u32; 2],
        scale_factor: f32,
        mask: Option<nana_ui_core::MaskImage>,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
        checkerboard: bool,
        zoom: f32,
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
                    fragment_clip.corner_radius,
                )
                .with_mask(mask)
                .with_checkerboard(checkerboard)
                .with_zoom(zoom),
        );
        primitive.prepare(
            &mut self.pipeline,
            device,
            queue,
            crate::geometry::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            affine,
            persp,
            scale_factor,
            physical_size,
            gpu_work,
        );
        let world = clip::transformed_aabb_projective(bounds, affine, persp);
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
