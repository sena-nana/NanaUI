use iced::widget::shader::{Pipeline as _, Primitive as _, Viewport};
use iced::{Rectangle, Size};

use super::clip::LogicalRect;
use crate::gpu_texture::{GpuTexturePipeline, GpuTexturePrimitive, HostTextureLayer};
use crate::{HostTextureBinding, PhysicalRect};

pub(super) struct HostTexturePipeline {
    pipeline: GpuTexturePipeline,
}

pub(super) struct PreparedHostTexture {
    primitive: GpuTexturePrimitive,
    clip: Rectangle<u32>,
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
        physical_size: [u32; 2],
        scale_factor: f32,
    ) -> PreparedHostTexture {
        let primitive = GpuTexturePrimitive::from_scene(
            node,
            slot,
            HostTextureLayer::from_binding(binding).with_opacity(opacity),
        );
        let viewport = Viewport::with_physical_size(
            Size::new(physical_size[0], physical_size[1]),
            iced::advanced::renderer::Scale {
                window: scale_factor,
                application: 1.0,
            },
        );
        primitive.prepare(
            &mut self.pipeline,
            device,
            queue,
            &Rectangle {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            &viewport,
        );
        PreparedHostTexture {
            primitive,
            clip: Rectangle {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
            },
        }
    }

    pub(super) fn render(
        &self,
        prepared: &PreparedHostTexture,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        prepared
            .primitive
            .render(&self.pipeline, encoder, target, &prepared.clip);
    }

    pub(super) fn trim(&mut self) {
        self.pipeline.trim();
    }
}
