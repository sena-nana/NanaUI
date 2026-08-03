use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

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

struct LayerUniform {
    opacity: vec4<f32>,
}

@group(0) @binding(2)
var<uniform> layer: LayerUniform;

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
    return textureSample(source, source_sampler, input.uv) * layer.opacity.x;
}
"#;

/// A stable handle to a host-owned, filterable 2D WGPU texture.
///
/// The handle remains valid while the host updates the content in place or
/// replaces the underlying view. `generation` changes only when the view is
/// replaced, while `version` changes for every content invalidation.
#[derive(Debug, Clone)]
pub struct HostTexture {
    state: Arc<HostTextureState>,
}

#[derive(Debug)]
struct HostTextureState {
    id: u64,
    generation: AtomicU64,
    version: AtomicU64,
    view: RwLock<wgpu::TextureView>,
}

#[derive(Debug)]
struct HostTextureSnapshot {
    id: u64,
    generation: u64,
    view: wgpu::TextureView,
}

impl HostTexture {
    pub fn from_wgpu(id: u64, generation: u64, view: wgpu::TextureView) -> Self {
        Self {
            state: Arc::new(HostTextureState {
                id,
                generation: AtomicU64::new(generation),
                version: AtomicU64::new(generation),
                view: RwLock::new(view),
            }),
        }
    }

    pub fn id(&self) -> u64 {
        self.state.id
    }

    pub fn generation(&self) -> u64 {
        self.state.generation.load(Ordering::Acquire)
    }

    /// Returns the monotonically increasing content version.
    pub fn version(&self) -> u64 {
        self.state.version.load(Ordering::Acquire)
    }

    /// Marks the current view as containing new host-rendered content.
    pub fn invalidate(&self) -> u64 {
        self.state.version.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Replaces the sampled view and invalidates the content in one operation.
    ///
    /// The stable handle is intentionally retained so an existing Iced tree
    /// observes the replacement without rebuilding its widgets or layout.
    pub fn replace_view(&self, view: wgpu::TextureView) -> u64 {
        *self.state.view.write().expect("host texture view lock") = view;
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        self.invalidate()
    }

    fn snapshot(&self) -> HostTextureSnapshot {
        HostTextureSnapshot {
            id: self.id(),
            generation: self.generation(),
            view: self
                .state
                .view
                .read()
                .expect("host texture view lock")
                .clone(),
        }
    }
}

/// Presentation properties for one host texture layer.
///
/// Layer order follows the order of the surrounding Iced elements (for
/// example, the order of a `stack!`), so composition remains owned by the UI
/// tree rather than by a second renderer.
#[derive(Debug, Clone)]
pub struct HostTextureLayer {
    texture: HostTexture,
    opacity: f32,
    clip: Option<LogicalRect>,
}

impl HostTextureLayer {
    pub const fn new(texture: HostTexture) -> Self {
        Self {
            texture,
            opacity: 1.0,
            clip: None,
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = finite_opacity(opacity);
        self
    }

    pub fn with_clip(mut self, clip: LogicalRect) -> Self {
        // Clip rectangles use the same window-logical coordinate space as the
        // Iced layout bounds and are intersected with the parent clip at draw.
        self.clip = Some(clip);
        self
    }

    pub const fn texture(&self) -> &HostTexture {
        &self.texture
    }

    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    pub const fn clip(&self) -> Option<LogicalRect> {
        self.clip
    }
}

/// Displays a host texture directly inside an Iced layout region.
#[derive(Debug, Clone)]
pub struct GpuTextureView {
    layer: HostTextureLayer,
}

impl GpuTextureView {
    pub const fn new(texture: HostTexture) -> Self {
        Self {
            layer: HostTextureLayer::new(texture),
        }
    }

    pub fn from_layer(layer: HostTextureLayer) -> Self {
        Self { layer }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.layer.opacity = finite_opacity(opacity);
        self
    }

    pub fn with_clip(mut self, clip: LogicalRect) -> Self {
        self.layer.clip = Some(clip);
        self
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
            layer: self.layer.clone(),
            logical_bounds: LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct GpuTexturePrimitive {
    layer: HostTextureLayer,
    logical_bounds: LogicalRect,
}

impl shader::Primitive for GpuTexturePrimitive {
    type Pipeline = GpuTexturePipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let texture = self.layer.texture.snapshot();
        let key = TextureKey::new(&self.layer);
        let slot = RenderSlot::new(texture.id, self.logical_bounds, viewport.scale_factor());
        let clip = self
            .layer
            .clip
            .map(|clip| RenderSlot::new(texture.id, clip, viewport.scale_factor()).physical);
        let needs_rebind = pipeline
            .textures
            .get(&key)
            .is_none_or(|prepared| prepared.generation != texture.generation);

        if needs_rebind {
            let layer_uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui host texture layer uniform"),
                size: std::mem::size_of::<LayerUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &layer_uniform,
                0,
                bytemuck::bytes_of(&LayerUniform {
                    opacity: [self.layer.opacity, 0.0, 0.0, 0.0],
                }),
            );
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui host texture bind group"),
                layout: &pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: layer_uniform.as_entire_binding(),
                    },
                ],
            });
            pipeline.textures.insert(
                key,
                PreparedTexture {
                    generation: texture.generation,
                    bind_group,
                    slot,
                    clip,
                    _layer_uniform: layer_uniform,
                    used: true,
                },
            );
        } else if let Some(prepared) = pipeline.textures.get_mut(&key) {
            prepared.slot = slot;
            prepared.clip = clip;
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
        let key = TextureKey::new(&self.layer);
        let Some(texture) = pipeline.textures.get(&key) else {
            return;
        };
        let clip_bounds = crate::geometry::PhysicalRect {
            x: clip_bounds.x,
            y: clip_bounds.y,
            width: clip_bounds.width,
            height: clip_bounds.height,
        };
        let layer_bounds = texture.clip.map_or(texture.slot.physical, |clip| {
            intersect_physical(texture.slot.physical, clip)
        });
        let bounds = intersect_physical(layer_bounds, clip_bounds);
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
            texture.slot.physical.x as f32,
            texture.slot.physical.y as f32,
            texture.slot.physical.width as f32,
            texture.slot.physical.height as f32,
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
    textures: HashMap<TextureKey, PreparedTexture>,
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
    clip: Option<crate::geometry::PhysicalRect>,
    _layer_uniform: wgpu::Buffer,
    used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextureKey {
    id: u64,
    opacity: u32,
    clip: Option<[u32; 4]>,
}

impl TextureKey {
    fn new(layer: &HostTextureLayer) -> Self {
        Self {
            id: layer.texture.id(),
            opacity: layer.opacity.to_bits(),
            clip: layer.clip.map(|clip| {
                [
                    clip.x.to_bits(),
                    clip.y.to_bits(),
                    clip.width.to_bits(),
                    clip.height.to_bits(),
                ]
            }),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    opacity: [f32; 4],
}

fn finite_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn intersect_physical(
    bounds: crate::geometry::PhysicalRect,
    clip: crate::geometry::PhysicalRect,
) -> crate::geometry::PhysicalRect {
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
    crate::geometry::PhysicalRect {
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
