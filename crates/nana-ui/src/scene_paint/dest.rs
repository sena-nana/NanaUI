//! Dest-local color target; MSAA is only for frames without GPU nodes.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DestPassCounts {
    pub color: u32,
    pub msaa: u32,
    pub blit: u32,
    pub msaa_allocated: bool,
}

pub(super) struct DestTarget {
    pub width: u32,
    pub height: u32,
    pub msaa_allocated: bool,
    msaa: Option<wgpu::TextureView>,
    color_view: wgpu::TextureView,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
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
        Self {
            width,
            height,
            msaa_allocated: want_msaa,
            msaa,
            color_view,
            blit_pipeline,
            blit_bind_group,
        }
    }

    pub(super) fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
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
