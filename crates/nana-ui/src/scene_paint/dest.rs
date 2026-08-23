//! Dest-local color target; MSAA is only for frames without GPU nodes.

use std::num::NonZeroU64;

use super::clip::FragmentClip;

const GROUP_UNIFORM_STRIDE: u64 = 256;
const GROUP_UNIFORM_SLOTS: u64 = 64;
const GROUP_UNIFORM_SIZE: u64 = 64;

#[derive(Debug, Clone, Copy)]
pub(super) struct GroupSlot {
    pub opacity: f32,
    pub clip: FragmentClip,
}

impl GroupSlot {
    pub fn opacity(opacity: f32) -> Self {
        Self {
            opacity,
            clip: FragmentClip::PASS,
        }
    }

    pub fn clip(clip: FragmentClip) -> Self {
        Self {
            opacity: 1.0,
            clip,
        }
    }
}

fn pack_group_slot(slot: &GroupSlot) -> [u8; GROUP_UNIFORM_SIZE as usize] {
    let mut bytes = [0u8; GROUP_UNIFORM_SIZE as usize];
    let fields = [
        (0, slot.opacity),
        (16, slot.clip.rect[0]),
        (20, slot.clip.rect[1]),
        (24, slot.clip.rect[2]),
        (28, slot.clip.rect[3]),
        (32, slot.clip.inv_abcd[0]),
        (36, slot.clip.inv_abcd[1]),
        (40, slot.clip.inv_abcd[2]),
        (44, slot.clip.inv_abcd[3]),
        (48, slot.clip.inv_ef[0]),
        (52, slot.clip.inv_ef[1]),
    ];
    for (offset, value) in fields {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DestPassCounts {
    pub color: u32,
    pub msaa: u32,
    pub blit: u32,
    pub group: u32,
    pub msaa_allocated: bool,
}

struct GroupLayer {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub(super) struct DestTarget {
    pub width: u32,
    pub height: u32,
    pub msaa_allocated: bool,
    format: wgpu::TextureFormat,
    msaa: Option<wgpu::TextureView>,
    color_view: wgpu::TextureView,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
    group_layers: Vec<GroupLayer>,
    group_pipeline: wgpu::RenderPipeline,
    group_bind_layout: wgpu::BindGroupLayout,
    group_sampler: wgpu::Sampler,
    group_uniforms: wgpu::Buffer,
    group_uniform_slots: u64,
}

impl DestTarget {
    pub(super) fn ensure(
        current: &mut Option<Self>,
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        want_msaa: bool,
    ) {
        if current.as_ref().is_some_and(|target| {
            target.width == width && target.height == height && target.msaa_allocated == want_msaa
        }) {
            return;
        }
        *current = Some(Self::new(
            device,
            pipeline_cache,
            format,
            width,
            height,
            want_msaa,
        ));
    }

    fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        want_msaa: bool,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let msaa = want_msaa.then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("nana-ui.scene.dest.msaa"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 4,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui.scene.dest.color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.dest.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.dest.blit.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.dest.blit.bind"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.dest.blit.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader/blit.wgsl"
            ))),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.dest.blit.pipeline"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.dest.blit.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });
        let group_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.group.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let group_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.group.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(GROUP_UNIFORM_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let group_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.group.uniforms"),
            size: GROUP_UNIFORM_STRIDE * GROUP_UNIFORM_SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.group.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("shader/color.wgsl"),
                "\n",
                include_str!("shader/layer.wgsl"),
            ))),
        });
        let group_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("nana-ui.scene.group.pipeline"),
                bind_group_layouts: &[Some(&group_bind_layout)],
                immediate_size: 0,
            });
        let group_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.group.pipeline"),
            layout: Some(&group_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &group_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &group_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });
        Self {
            width,
            height,
            msaa_allocated: want_msaa,
            format,
            msaa,
            color_view,
            blit_pipeline,
            blit_bind_group,
            group_layers: Vec::new(),
            group_pipeline,
            group_bind_layout,
            group_sampler,
            group_uniforms,
            group_uniform_slots: GROUP_UNIFORM_SLOTS,
        }
    }

    pub(super) fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub(super) fn group_view(&self, layer: usize) -> &wgpu::TextureView {
        &self.group_layers[layer].view
    }

    pub(super) fn prepare_groups(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: usize,
        slots: &[GroupSlot],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        while self.group_layers.len() < layers {
            self.push_group_layer(device);
        }
        let needed = (slots.len() as u64).max(1);
        if needed > self.group_uniform_slots {
            self.resize_group_uniforms(device, needed);
        }
        for (index, slot) in slots.iter().enumerate() {
            let bytes = pack_group_slot(slot);
            queue.write_buffer(
                &self.group_uniforms,
                index as u64 * GROUP_UNIFORM_STRIDE,
                &bytes,
            );
            if let Some(work) = gpu_work {
                work.record_upload(bytes.len());
            }
        }
    }

    fn resize_group_uniforms(&mut self, device: &wgpu::Device, slots: u64) {
        let slots = slots.max(1);
        self.group_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.group.uniforms"),
            size: GROUP_UNIFORM_STRIDE * slots,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.group_uniform_slots = slots;
        let layers = std::mem::take(&mut self.group_layers);
        let count = layers.len();
        drop(layers);
        for _ in 0..count {
            self.push_group_layer(device);
        }
    }

    fn push_group_layer(&mut self, device: &wgpu::Device) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui.scene.group.color"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.group.bind"),
            layout: &self.group_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.group_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.group_uniforms,
                        offset: 0,
                        size: NonZeroU64::new(GROUP_UNIFORM_SIZE),
                    }),
                },
            ],
        });
        self.group_layers.push(GroupLayer {
            _texture: texture,
            view,
            bind_group,
        });
    }

    pub(super) fn begin_group_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        layer: usize,
        load: wgpu::LoadOp<wgpu::Color>,
        counts: &mut DestPassCounts,
    ) -> wgpu::RenderPass<'a> {
        counts.group = counts.group.saturating_add(1);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui.scene.group"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.group_layers[layer].view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    pub(super) fn composite_group(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        layer: usize,
        slot: u32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let max_slot = self.group_uniform_slots.saturating_sub(1) as u32;
        let slot = slot.min(max_slot);
        let offset = (slot as u64 * GROUP_UNIFORM_STRIDE) as u32;
        pass.set_pipeline(&self.group_pipeline);
        pass.set_bind_group(0, &self.group_layers[layer].bind_group, &[offset]);
        pass.draw(0..3, 0..1);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }

    pub(super) fn begin_color_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<wgpu::Color>,
        counts: &mut DestPassCounts,
    ) -> wgpu::RenderPass<'a> {
        counts.color = counts.color.saturating_add(1);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui.scene.dest.color"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    pub(super) fn begin_msaa_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        clear: wgpu::Color,
        counts: &mut DestPassCounts,
    ) -> wgpu::RenderPass<'a> {
        counts.msaa = counts.msaa.saturating_add(1);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui.scene.dest.msaa"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.msaa.as_ref().expect("UI-only dest allocates 4x MSAA"),
                depth_slice: None,
                resolve_target: Some(&self.color_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        dest_x: f32,
        dest_y: f32,
        window: [u32; 2],
        clear_window: Option<wgpu::Color>,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
        counts: &mut DestPassCounts,
    ) {
        counts.blit = counts.blit.saturating_add(1);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui.scene.dest.blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: clear_window.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let x = dest_x.max(0.0);
        let y = dest_y.max(0.0);
        let max_w = window[0] as f32 - x;
        let max_h = window[1] as f32 - y;
        if max_w < 1.0 || max_h < 1.0 {
            return;
        }
        pass.set_viewport(
            x,
            y,
            (self.width as f32).min(max_w),
            (self.height as f32).min(max_h),
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &self.blit_bind_group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}
