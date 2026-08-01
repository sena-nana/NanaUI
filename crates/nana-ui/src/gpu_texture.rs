use std::collections::HashMap;

use iced::wgpu;
use iced::widget::{container, responsive, shader};
use iced::{Element, Length, Rectangle};

use crate::geometry::LogicalRect;
use crate::gpu_view::RenderSlot;

const SOURCE: &str = r#"
@group(0) @binding(0)
var source: texture_2d<f32>;

@group(0) @binding(1)
var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let raw_uv = positions[index] * 0.5 + vec2<f32>(0.5);
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    output.uv = vec2<f32>(raw_uv.x, 1.0 - raw_uv.y);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, input.uv);
}
"#;

/// A ref-counted view of a host-owned, filterable 2D WGPU texture.
///
/// `generation` must change whenever the host replaces the underlying view.
#[derive(Debug, Clone)]
pub struct HostTexture {
    id: u64,
    generation: u64,
    view: wgpu::TextureView,
}

impl HostTexture {
    pub fn from_wgpu(id: u64, generation: u64, view: wgpu::TextureView) -> Self {
        Self {
            id,
            generation,
            view,
        }
    }

    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Displays a host texture directly inside an Iced layout region.
#[derive(Debug, Clone)]
pub struct GpuTextureView {
    texture: HostTexture,
}

impl GpuTextureView {
    pub const fn new(texture: HostTexture) -> Self {
        Self { texture }
    }

    /// Fits the texture inside the available region while preserving its aspect ratio.
    pub fn contain<Message: 'static>(self, aspect_ratio: f32) -> Element<'static, Message> {
        let aspect_ratio = finite_aspect(aspect_ratio);
        responsive(move |size| {
            let (width, height) = contain_size(size.width, size.height, aspect_ratio);
            container(
                shader(self.clone())
                    .width(Length::Fixed(width))
                    .height(Length::Fixed(height)),
            )
            .center(Length::Fill)
        })
        .into()
    }
}

fn finite_aspect(aspect_ratio: f32) -> f32 {
    if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        1.0
    }
}

fn contain_size(width: f32, height: f32, aspect_ratio: f32) -> (f32, f32) {
    let width = width.max(0.0);
    let height = height.max(0.0);
    if width == 0.0 || height == 0.0 {
        return (width, height);
    }
    if width / height > aspect_ratio {
        (height * aspect_ratio, height)
    } else {
        (width, width / aspect_ratio)
    }
}

impl<Message> shader::Program<Message> for GpuTextureView {
    type State = ();
    type Primitive = GpuTexturePrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        GpuTexturePrimitive {
            texture: self.texture.clone(),
            logical_bounds: LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct GpuTexturePrimitive {
    texture: HostTexture,
    logical_bounds: LogicalRect,
}

impl shader::Primitive for GpuTexturePrimitive {
    type Pipeline = GpuTexturePipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let slot = RenderSlot::new(
            self.texture.id,
            self.logical_bounds,
            viewport.scale_factor(),
        );
        let needs_rebind = pipeline
            .textures
            .get(&self.texture.id)
            .is_none_or(|prepared| prepared.generation != self.texture.generation);

        if needs_rebind {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui host texture bind group"),
                layout: &pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            });
            pipeline.textures.insert(
                self.texture.id,
                PreparedTexture {
                    generation: self.texture.generation,
                    bind_group,
                    slot,
                    used: true,
                },
            );
        } else if let Some(prepared) = pipeline.textures.get_mut(&self.texture.id) {
            prepared.slot = slot;
            prepared.used = true;
        }
    }

    fn draw(&self, _pipeline: &Self::Pipeline, _render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        false
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(texture) = pipeline.textures.get(&self.texture.id) else {
            return;
        };
        let bounds = intersect_physical(texture.slot.physical, *clip_bounds);
        if bounds.width == 0 || bounds.height == 0 {
            return;
        }
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui host texture render pass"),
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
        render_pass.set_viewport(
            bounds.x as f32,
            bounds.y as f32,
            bounds.width as f32,
            bounds.height as f32,
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &texture.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[doc(hidden)]
pub struct GpuTexturePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<u64, PreparedTexture>,
}

impl shader::Pipeline for GpuTexturePipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui host texture shader"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui host texture bind group layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui host texture pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui host texture pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
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
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui host texture sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            textures: HashMap::new(),
        }
    }

    fn trim(&mut self) {
        self.textures.retain(|_, texture| {
            let retain = texture.used;
            texture.used = false;
            retain
        });
    }
}

struct PreparedTexture {
    generation: u64,
    bind_group: wgpu::BindGroup,
    slot: RenderSlot,
    used: bool,
}

fn intersect_physical(
    bounds: crate::geometry::PhysicalRect,
    clip: Rectangle<u32>,
) -> Rectangle<u32> {
    let left = bounds.x.max(clip.x);
    let top = bounds.y.max(clip.y);
    let right = bounds
        .x
        .saturating_add(bounds.width)
        .min(clip.x.saturating_add(clip.width));
    let bottom = bounds
        .y
        .saturating_add(bounds.height)
        .min(clip.y.saturating_add(clip.height));
    Rectangle {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

#[cfg(test)]
mod tests {
    use super::{contain_size, finite_aspect};

    #[test]
    fn contain_preserves_aspect_inside_wide_and_tall_regions() {
        assert_eq!(contain_size(800.0, 800.0, 16.0 / 9.0), (800.0, 450.0));
        assert_eq!(contain_size(1920.0, 540.0, 16.0 / 9.0), (960.0, 540.0));
        assert_eq!(finite_aspect(f32::NAN), 1.0);
    }
}
