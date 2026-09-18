//! The renderer's own text pipelines, shaders and per-frame GPU buffers.
//!
//! Two programs over one atlas bind group, because text arrives in two shapes
//! and one program would pay the wrong cost for both:
//!
//! - [`Axis`](SegmentKind::Axis): pixel-aligned glyphs under a translation.
//!   One 24-byte instance per glyph, four vertices, clipped by the scissor the
//!   batch already carries. This is essentially every glyph a shell draws.
//!   Nearest sampling: the quad lands on the texel grid, so filtering would
//!   only soften it.
//! - [`Affine`](SegmentKind::Affine): glyphs under a rotation, a scale or a
//!   rounded clip. Six vertices per glyph carrying the same homography and
//!   fragment clip as `Quad`, and linear sampling because the taps no longer
//!   line up with texels.
//!
//! Both sample the same pages through the same bind group, so a page that a
//! rotated label faulted in is the page an upright one hits.

use bytemuck::{Pod, Zeroable};

use super::atlas::GlyphAtlasManager;
use crate::scene_paint::clip;
use crate::scene_paint::color::orthographic;

/// Content type carried per glyph. Mirrors [`super::atlas::AtlasPageKind`];
/// the shader switches on it to pick which page the glyph came from.
pub(super) const CONTENT_MASK: u32 = 0;
pub(super) const CONTENT_COLOR: u32 = 1;

const INITIAL_INSTANCES: usize = 512;
const INITIAL_VERTICES: usize = 256;

const AXIS_SHADER: &str = concat!(
    include_str!("../shader/text_atlas.wgsl"),
    r#"
struct VsIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) origin: vec2<i32>,
    @location(1) dim: u32,
    @location(2) uv: u32,
    @location(3) color: u32,
    @location(4) content: u32,
}

struct VsOut {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) content: u32,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    let width = input.dim & 0xffffu;
    let height = (input.dim & 0xffff0000u) >> 16u;
    let corner = vec2<u32>(input.vertex & 1u, (input.vertex >> 1u) & 1u);
    let offset = vec2<u32>(width, height) * corner;
    let position = input.origin + vec2<i32>(offset);
    let texel = vec2<u32>(input.uv & 0xffffu, (input.uv & 0xffff0000u) >> 16u) + offset;

    var out: VsOut;
    out.position = globals.transform * vec4<f32>(vec2<f32>(position), 0.0, 1.0);
    out.color = unpack_srgb(input.color);
    out.uv = atlas_uv(texel, input.content);
    out.content = input.content;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    if input.content == 0u {
        let coverage = textureSampleLevel(mask_atlas, atlas_nearest, input.uv, 0.0).x;
        return vec4<f32>(input.color.rgb, input.color.a * coverage);
    }
    // A color bitmap carries its own color; only the run's alpha applies, so a
    // faded or shadowed emoji fades instead of painting at full strength.
    let sampled = textureSampleLevel(color_atlas, atlas_nearest, input.uv, 0.0);
    return vec4<f32>(sampled.rgb, sampled.a * input.color.a);
}
"#
);

const AFFINE_SHADER: &str = concat!(
    include_str!("../shader/color.wgsl"),
    include_str!("../shader/text_atlas.wgsl"),
    r#"
struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<u32>,
    @location(2) color: vec4<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec3<f32>,
    @location(6) content: u32,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec3<f32>,
    @location(6) @interpolate(flat) content: u32,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    out.position = globals.transform * vec4<f32>(input.position, 0.0, 1.0);
    out.uv = atlas_uv(input.uv, input.content);
    out.color = input.color;
    out.world_pos = input.position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;
    out.content = input.content;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    if !inside_fragment_clip(
        input.world_pos,
        input.clip_rect,
        input.clip_inv_abcd,
        input.clip_inv_ef.xy,
        input.clip_inv_ef.z,
        0u,
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    ) {
        discard;
    }
    if input.content == 0u {
        let coverage = textureSample(mask_atlas, atlas_linear, input.uv).x;
        return vec4<f32>(input.color.rgb, input.color.a * coverage);
    }
    let sampled = textureSample(color_atlas, atlas_linear, input.uv);
    return vec4<f32>(sampled.rgb, sampled.a * input.color.a);
}
"#
);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Globals {
    transform: [f32; 16],
}

/// One axis-aligned glyph: 24 bytes, against the 60 a vertex-per-corner
/// quad would cost. This compactness is the whole point of the instanced path.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct GlyphInstance {
    origin: [i32; 2],
    /// `width | height << 16`, in physical pixels.
    dim: u32,
    /// Atlas texel `x | y << 16`.
    uv: u32,
    /// sRGB `a << 24 | r << 16 | g << 8 | b`, linearized in the shader so the
    /// instance stays 20 bytes and the conversion matches every other pipeline.
    color: u32,
    content: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct AffineVertex {
    position: [f32; 2],
    /// Atlas texels, normalized in the vertex stage like the axis path's are.
    uv: [u32; 2],
    /// Already linear: this path builds few vertices and keeping the float
    /// color avoids quantizing a shadow's alpha ramp to 8 bits.
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 3],
    content: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SegmentKind {
    Axis,
    Affine,
}

/// One draw: a contiguous span of one kind that samples one pair of pages.
///
/// Spans are split rather than grouped when a page changes, so glyph order
/// inside a run is document order even across a page boundary.
#[derive(Clone, Copy, Debug)]
pub(super) struct DrawSegment {
    pub kind: SegmentKind,
    pub mask_page: u32,
    pub color_page: u32,
    pub first: u32,
    pub count: u32,
}

/// GPU state that belongs to one render target: the buffers this frame's
/// glyphs land in and the projection they are placed against.
pub(super) struct TextTargetGpu {
    instances: wgpu::Buffer,
    instance_capacity: usize,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    globals: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    uploaded_size: Option<[u32; 2]>,
    /// GPU allocations this target could not avoid this frame.
    allocations: usize,
}

pub(super) struct TextGpu {
    axis: wgpu::RenderPipeline,
    affine: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    pub(super) target: TextTargetGpu,
}

impl TextGpu {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas: &GlyphAtlasManager,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.text.globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.text.pipeline"),
            bind_group_layouts: &[Some(&globals_layout), Some(atlas.layout())],
            immediate_size: 0,
        });
        let axis_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.text.axis.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(AXIS_SHADER)),
        });
        let affine_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.text.affine.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(AFFINE_SHADER)),
        });
        let axis = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.text.axis.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &axis_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Sint32x2,
                        1 => Uint32,
                        2 => Uint32,
                        3 => Uint32,
                        4 => Uint32,
                    ),
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &axis_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let affine = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.text.affine.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &affine_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<AffineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Uint32x2,
                        2 => Float32x4,
                        3 => Float32x4,
                        4 => Float32x4,
                        5 => Float32x3,
                        6 => Uint32,
                    ),
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &affine_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let target = TextTargetGpu::new(device, &globals_layout);
        Self {
            axis,
            affine,
            globals_layout,
            target,
        }
    }

    pub(super) fn new_target(&self, device: &wgpu::Device) -> TextTargetGpu {
        TextTargetGpu::new(device, &self.globals_layout)
    }

    pub(super) fn take_allocations(&mut self) -> usize {
        std::mem::take(&mut self.target.allocations)
    }

    /// Write this frame's projection, instances and vertices.
    ///
    /// Only the blocks that differ from what this target already holds, so a
    /// repaint of unchanged text costs no queue traffic at all.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        physical_size: [u32; 2],
        instances: &[GlyphInstance],
        vertices: &[AffineVertex],
        uploaded_instances: &[GlyphInstance],
        uploaded_vertices: &[AffineVertex],
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let target = &mut self.target;
        if target.uploaded_size != Some(physical_size) {
            queue.write_buffer(
                &target.globals,
                0,
                bytemuck::bytes_of(&Globals {
                    transform: orthographic(physical_size[0], physical_size[1]),
                }),
            );
            target.uploaded_size = Some(physical_size);
            if let Some(work) = work {
                work.record_upload(std::mem::size_of::<Globals>());
            }
        }
        let mut bytes = 0;
        if !instances.is_empty() {
            let mut previous: &[GlyphInstance] = uploaded_instances;
            if instances.len() > target.instance_capacity {
                target.instance_capacity = instances.len().next_power_of_two();
                target.instances = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.text.instances"),
                    size: (target.instance_capacity * std::mem::size_of::<GlyphInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                target.allocations += 1;
                previous = &[];
            }
            bytes += crate::scene_paint::buffer_upload::upload_changed(
                queue,
                &target.instances,
                bytemuck::cast_slice(previous),
                bytemuck::cast_slice(instances),
            );
        }
        if !vertices.is_empty() {
            let mut previous: &[AffineVertex] = uploaded_vertices;
            if vertices.len() > target.vertex_capacity {
                target.vertex_capacity = vertices.len().next_power_of_two();
                target.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.text.vertices"),
                    size: (target.vertex_capacity * std::mem::size_of::<AffineVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                target.allocations += 1;
                previous = &[];
            }
            bytes += crate::scene_paint::buffer_upload::upload_changed(
                queue,
                &target.vertices,
                bytemuck::cast_slice(previous),
                bytemuck::cast_slice(vertices),
            );
        }
        if let Some(work) = work {
            work.record_upload(bytes);
        }
    }

    pub(super) fn draw_segment(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        segment: &DrawSegment,
        bind_group: &wgpu::BindGroup,
    ) {
        if segment.count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.target.globals_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        match segment.kind {
            SegmentKind::Axis => {
                pass.set_pipeline(&self.axis);
                pass.set_vertex_buffer(0, self.target.instances.slice(..));
                pass.draw(0..4, segment.first..segment.first + segment.count);
            }
            SegmentKind::Affine => {
                pass.set_pipeline(&self.affine);
                pass.set_vertex_buffer(0, self.target.vertices.slice(..));
                pass.draw(segment.first..segment.first + segment.count, 0..1);
            }
        }
    }
}

impl TextTargetGpu {
    fn new(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout) -> Self {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.text.globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.text.globals.bind"),
            layout: globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        Self {
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.instances"),
                size: (INITIAL_INSTANCES * std::mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            instance_capacity: INITIAL_INSTANCES,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.vertices"),
                size: (INITIAL_VERTICES * std::mem::size_of::<AffineVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_capacity: INITIAL_VERTICES,
            globals,
            globals_bind_group,
            uploaded_size: None,
            allocations: 0,
        }
    }
}

impl GlyphInstance {
    pub(super) fn new(
        origin: [i32; 2],
        size: [u32; 2],
        texel: [u32; 2],
        color: [f32; 4],
        content: u32,
    ) -> Self {
        Self {
            origin,
            dim: (size[0] & 0xffff) | ((size[1] & 0xffff) << 16),
            uv: (texel[0] & 0xffff) | ((texel[1] & 0xffff) << 16),
            color: pack_srgb(color),
            content,
        }
    }
}

impl AffineVertex {
    pub(super) fn new(
        position: [f32; 2],
        uv: [u32; 2],
        color: [f32; 4],
        clip: &clip::FragmentClip,
        content: u32,
    ) -> Self {
        Self {
            position,
            uv,
            color,
            clip_rect: clip.rect,
            clip_inv_abcd: clip.inv_abcd,
            clip_inv_ef: [clip.inv_ef[0], clip.inv_ef[1], clip.corner_radius],
            content,
        }
    }
}

fn pack_srgb(color: [f32; 4]) -> u32 {
    let [r, g, b, a] = crate::scene_paint::color::to_rgba8(color);
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instance_packs_the_fields_the_shader_unpacks() {
        let instance = GlyphInstance::new(
            [3, -4],
            [17, 260],
            [40, 1030],
            [1.0, 0.0, 0.0, 0.5],
            CONTENT_MASK,
        );
        assert_eq!(instance.origin, [3, -4]);
        assert_eq!(instance.dim & 0xffff, 17);
        assert_eq!((instance.dim >> 16) & 0xffff, 260);
        assert_eq!(instance.uv & 0xffff, 40);
        assert_eq!((instance.uv >> 16) & 0xffff, 1030);
        assert_eq!(instance.color >> 16 & 0xff, 255, "red survives the pack");
        assert_eq!(instance.color >> 24, 128, "alpha is the high byte");
        assert_eq!(instance.content, CONTENT_MASK);
    }

    #[test]
    fn the_instance_stays_the_size_the_axis_path_is_built_around() {
        assert_eq!(std::mem::size_of::<GlyphInstance>(), 24);
        assert!(
            std::mem::size_of::<GlyphInstance>() * 4 < std::mem::size_of::<AffineVertex>() * 6,
            "an instanced glyph must stay cheaper than six transformed vertices"
        );
        assert_eq!(CONTENT_COLOR, 1);
    }
}
