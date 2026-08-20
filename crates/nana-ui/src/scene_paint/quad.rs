use bytemuck::{Pod, Zeroable};
use nana_ui_runtime::ComponentElevation;

use super::clip::LogicalRect;
use super::color::{orthographic, pack_linear, with_opacity};
use crate::PhysicalRect;

const INITIAL_INSTANCES: usize = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SolidInstance {
    color: [f32; 4],
    position: [f32; 2],
    size: [f32; 2],
    border_color: [f32; 4],
    border_radius: [f32; 4],
    border_width: f32,
    shadow_color: [f32; 4],
    shadow_offset: [f32; 2],
    shadow_blur_radius: f32,
    snap: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
    scale: f32,
    _padding: [f32; 3],
}

pub(super) struct QuadPipeline {
    pipeline: wgpu::RenderPipeline,
    pipeline_msaa: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    pending: Vec<SolidInstance>,
}

impl QuadPipeline {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.quad.solid.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("shader/color.wgsl"),
                "\n",
                include_str!("shader/quad.wgsl"),
                "\n",
                include_str!("shader/vertex.wgsl"),
                "\n",
                include_str!("shader/quad_solid.wgsl"),
            ))),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.quad.uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.quad.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.quad.uniforms.bind_group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.quad.solid.pipeline"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = solid_pipeline(device, &shader, &layout, format, 1);
        let pipeline_msaa = solid_pipeline(device, &shader, &layout, format, 4);
        let instance_capacity = INITIAL_INSTANCES;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.quad.instances"),
            size: (instance_capacity * std::mem::size_of::<SolidInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            pipeline_msaa,
            bind_group,
            uniforms,
            instances,
            instance_capacity,
            pending: Vec::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        bounds: LogicalRect,
        clip: LogicalRect,
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: f32,
        shadow: Option<ComponentElevation>,
        opacity: f32,
    ) -> Option<u32> {
        let bounds = bounds.intersection(clip)?;
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return None;
        }
        let index = self.pending.len() as u32;
        self.pending.push(SolidInstance {
            color: pack_linear(with_opacity(
                background.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                opacity,
            )),
            position: [bounds.x, bounds.y],
            size: [bounds.width, bounds.height],
            border_color: pack_linear(with_opacity(
                border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                opacity,
            )),
            border_radius: [corner_radius; 4],
            border_width,
            shadow_color: pack_linear(with_opacity(
                shadow
                    .map(|shadow| shadow.color)
                    .unwrap_or([0.0, 0.0, 0.0, 0.0]),
                opacity,
            )),
            shadow_offset: [0.0, shadow.map(|shadow| shadow.offset_y).unwrap_or(0.0)],
            shadow_blur_radius: shadow.map(|shadow| shadow.blur_radius).unwrap_or(0.0),
            snap: 1,
        });
        Some(index)
    }

    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        physical_size: [u32; 2],
        scale_factor: f32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let uniforms = Uniforms {
            transform: orthographic(physical_size[0], physical_size[1]),
            scale: scale_factor,
            _padding: [0.0; 3],
        };
        let uniform_bytes = bytemuck::bytes_of(&uniforms);
        queue.write_buffer(&self.uniforms, 0, uniform_bytes);
        if let Some(work) = gpu_work {
            work.record_upload(uniform_bytes.len());
        }
        if self.pending.is_empty() {
            return;
        }
        if self.pending.len() > self.instance_capacity {
            self.instance_capacity = self.pending.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.quad.instances"),
                size: (self.instance_capacity * std::mem::size_of::<SolidInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(work) = gpu_work {
                work.record_realloc();
            }
        }
        let instance_bytes = bytemuck::cast_slice(&self.pending);
        queue.write_buffer(&self.instances, 0, instance_bytes);
        if let Some(work) = gpu_work {
            work.record_upload(instance_bytes.len());
            work.record_batch_rebuild();
        }
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: std::ops::Range<u32>,
        scissor: PhysicalRect,
        sample_count: u32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        if range.start >= range.end {
            return;
        }
        pass.set_pipeline(if sample_count > 1 {
            &self.pipeline_msaa
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw(0..6, range);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}

fn solid_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("nana-ui.scene.quad.solid.pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("solid_vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SolidInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array!(
                    0 => Float32x4,
                    1 => Float32x2,
                    2 => Float32x2,
                    3 => Float32x4,
                    4 => Float32x4,
                    5 => Float32,
                    6 => Float32x4,
                    7 => Float32x2,
                    8 => Float32,
                    9 => Uint32,
                ),
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("solid_fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Cw,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}
