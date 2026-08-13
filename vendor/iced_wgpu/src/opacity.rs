use crate::wgpu;
use wgpu::util::DeviceExt;

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu opacity group layout"),
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
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu opacity group shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                struct Params {
                    inverse_x: vec4<f32>,
                    inverse_y: vec4<f32>,
                    source_size: vec4<f32>,
                }
                @group(0) @binding(0) var source: texture_2d<f32>;
                @group(0) @binding(1) var source_sampler: sampler;
                @group(0) @binding(2) var<uniform> params: Params;

                struct VertexOut {
                    @builtin(position) position: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                }

                @vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOut {
                    let positions = array<vec2<f32>, 3>(
                        vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0)
                    );
                    var out: VertexOut;
                    out.position = vec4(positions[index], 0.0, 1.0);
                    out.uv = positions[index] * vec2(0.5, -0.5) + vec2(0.5, 0.5);
                    return out;
                }

                @fragment fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
                    let destination = vec3(in.position.xy, 1.0);
                    let source_pixel = vec2(
                        dot(destination, params.inverse_x.xyz),
                        dot(destination, params.inverse_y.xyz),
                    );
                    if (source_pixel.x < 0.0 || source_pixel.y < 0.0 ||
                        source_pixel.x >= params.source_size.x ||
                        source_pixel.y >= params.source_size.y) {
                        discard;
                    }
                    let uv = source_pixel / params.source_size.xy;
                    return textureSample(source, source_sampler, uv) * params.inverse_x.w;
                }
                "#
                .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("iced_wgpu opacity group pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu opacity group pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("iced_wgpu opacity group sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    pub fn composite(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        opacity: f32,
        affine: [f32; 6],
        scale_factor: f32,
        source_size: crate::core::Size<u32>,
        scissor: crate::core::Rectangle<u32>,
    ) {
        let [a, b, c, d, e, f] = affine;
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return;
        }
        let physical_e = e * scale_factor;
        let physical_f = f * scale_factor;
        let inverse = [
            d / determinant,
            -c / determinant,
            (c * physical_f - d * physical_e) / determinant,
            opacity,
            -b / determinant,
            a / determinant,
            (b * physical_e - a * physical_f) / determinant,
            0.0,
            source_size.width as f32,
            source_size.height as f32,
            0.0,
            0.0,
        ];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("iced_wgpu opacity group uniform"),
            contents: bytemuck::cast_slice(&inverse),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu opacity group bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("iced_wgpu opacity group composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw(0..3, 0..1);
    }
}
