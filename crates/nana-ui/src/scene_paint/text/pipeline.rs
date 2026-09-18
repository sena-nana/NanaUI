//! The renderer's own text pipeline, shader and per-target GPU buffers.
//!
//! One program over one atlas bind group. A glyph is always the same 24-byte
//! instance — four vertices, its origin relative to its run, its rectangle in
//! the atlas — and everything about *how* that paragraph reaches the screen
//! lives in two side tables the instances only name:
//!
//! - [`TextRunGpu`]: the run's whole-pixel origin, its color, its opacity and
//!   which presentation it paints under. One per text draw.
//! - [`TextPresentationGpu`]: the transform and the clip. Deduplicated, because
//!   a shell's labels nearly all share one identity transform and one clip.
//!
//! That split is what makes moving, fading or recoloring text a patch of tens
//! of bytes instead of a rebuild of every glyph: [`super::entry`] hands the
//! same instance range back frame after frame and only rewrites a table row.
//!
//! Upright and transformed text are the same program because the difference
//! between them is two bits in the run: whether each corner goes through the
//! homography, and whether the sampler is bilinear. A page a rotated label
//! faulted in is the page an upright one hits.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};

use super::atlas::GlyphAtlasManager;
use crate::scene_paint::clip;
use crate::scene_paint::color::orthographic;

/// Content type carried per glyph. Mirrors [`super::atlas::AtlasPageKind`];
/// the shader switches on it to pick which page the glyph came from.
pub(super) const CONTENT_MASK: u32 = 0;
pub(super) const CONTENT_COLOR: u32 = 1;
/// The instance carries its own color instead of inheriting the run's. Set for
/// a rich span, clear for the overwhelming majority of glyphs — which is what
/// lets a plain label's color change be a four-float write.
pub(super) const INSTANCE_OWN_COLOR: u32 = 2;
/// Bits above these two are the run index.
pub(super) const INSTANCE_RUN_SHIFT: u32 = 2;

/// Bilinear sampling: the quad no longer lands on the texel grid.
pub(super) const RUN_LINEAR: u32 = 1;
/// The run carries a clip the scissor cannot express.
pub(super) const RUN_CLIP: u32 = 2;
/// More than a translation, so each corner goes through the homography.
pub(super) const RUN_PROJECT: u32 = 4;

const INITIAL_INSTANCES: usize = 512;
const INITIAL_RUNS: usize = 64;
const INITIAL_PRESENTATIONS: usize = 8;

const TEXT_SHADER: &str = concat!(
    include_str!("../shader/color.wgsl"),
    include_str!("../shader/text_atlas.wgsl"),
    r#"
struct VsIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) origin: vec2<i32>,
    @location(1) dim: u32,
    @location(2) uv: u32,
    @location(3) color: u32,
    @location(4) control: u32,
}

struct VsOut {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_pos: vec2<f32>,
    @location(3) @interpolate(flat) content: u32,
    // `presentation << 3 | flags`. Carried rather than re-read: the vertex
    // stage already has the run, and these are the same for every fragment of
    // a paragraph, so reading the row again per fragment would be a dependent
    // load for a value that cannot vary.
    @location(4) @interpolate(flat) run_flags: u32,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    let run = text_runs[input.control >> 2u];
    let content = input.control & 1u;
    let width = input.dim & 0xffffu;
    let height = (input.dim & 0xffff0000u) >> 16u;
    let corner = vec2<u32>(input.vertex & 1u, (input.vertex >> 1u) & 1u);
    let offset = vec2<u32>(width, height) * corner;
    let local = vec2<f32>(input.origin + vec2<i32>(offset));
    let texel = vec2<u32>(input.uv & 0xffffu, (input.uv & 0xffff0000u) >> 16u) + offset;

    var color = run.color;
    if (input.control & 2u) != 0u {
        color = unpack_srgb(input.color);
    }
    color.a = color.a * run.opacity;

    let world = text_world_position(run, local);
    var out: VsOut;
    out.position = globals.transform * vec4<f32>(world, 0.0, 1.0);
    out.color = color;
    out.uv = atlas_uv(texel, content);
    out.world_pos = world;
    out.content = content;
    out.run_flags = (run.presentation << 3u) | (run.flags & 7u);
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let flags = input.run_flags & 7u;
    if (flags & RUN_CLIP) != 0u {
        let presentation = text_presentations[input.run_flags >> 3u];
        if !inside_fragment_clip(
            input.world_pos,
            presentation.clip_rect,
            presentation.clip_inv_abcd,
            presentation.clip_inv_ef.xy,
            presentation.clip_inv_ef.z,
            presentation.polygon_count,
            presentation.polygon[0],
            presentation.polygon[1],
            presentation.polygon[2],
            presentation.polygon[3],
        ) {
            discard;
        }
    }
    let linear = (flags & RUN_LINEAR) != 0u;
    if input.content == 0u {
        var coverage = 0.0;
        if linear {
            coverage = textureSampleLevel(mask_atlas, atlas_linear, input.uv, 0.0).x;
        } else {
            coverage = textureSampleLevel(mask_atlas, atlas_nearest, input.uv, 0.0).x;
        }
        return vec4<f32>(input.color.rgb, input.color.a * coverage);
    }
    // A color bitmap carries its own color; only the run's alpha applies, so a
    // faded or shadowed emoji fades instead of painting at full strength.
    var sampled = vec4<f32>(0.0);
    if linear {
        sampled = textureSampleLevel(color_atlas, atlas_linear, input.uv, 0.0);
    } else {
        sampled = textureSampleLevel(color_atlas, atlas_nearest, input.uv, 0.0);
    }
    return vec4<f32>(sampled.rgb, sampled.a * input.color.a);
}
"#
);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Globals {
    transform: [f32; 16],
}

/// One glyph: 24 bytes, against the 60 a vertex-per-corner quad would cost.
/// This compactness is the whole point of the instanced path, and keeping
/// presentation out of it is what lets the bytes be retained across frames.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub(super) struct GlyphInstance {
    /// Quad top-left in physical pixels, relative to the run's origin.
    origin: [i32; 2],
    /// `width | height << 16`, in physical pixels.
    dim: u32,
    /// Atlas texel `x | y << 16`.
    uv: u32,
    /// sRGB `a << 24 | r << 16 | g << 8 | b`, linearized in the shader. Only
    /// read when [`INSTANCE_OWN_COLOR`] is set.
    color: u32,
    /// `content | own-color | run << 2`.
    control: u32,
}

/// One text draw's presentation. 48 bytes; see the module docs.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(super) struct TextRunGpu {
    pub origin: [f32; 2],
    pub presentation: u32,
    pub flags: u32,
    /// Linear RGB with its own alpha, opacity not yet applied.
    pub color: [f32; 4],
    pub opacity: f32,
    pad: [f32; 3],
}

/// The transform and clip a run paints under. 160 bytes, deduplicated.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(super) struct TextPresentationGpu {
    affine: [f32; 4],
    project: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 4],
    polygon: [[f32; 4]; 4],
    polygon_count: u32,
    pad: [u32; 3],
}

/// One draw: a contiguous span of instances that samples one pair of pages.
///
/// Spans are split rather than grouped when a page changes, so glyph order
/// inside a run is document order even across a page boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DrawSegment {
    pub mask_page: u32,
    pub color_page: u32,
    pub first: u32,
    pub count: u32,
}

/// GPU state that belongs to one render target: the buffers this frame's
/// glyphs and their presentation land in, and the projection they are placed
/// against.
pub(super) struct TextTargetGpu {
    instances: wgpu::Buffer,
    instance_capacity: usize,
    runs: wgpu::Buffer,
    run_capacity: usize,
    presentations: wgpu::Buffer,
    presentation_capacity: usize,
    globals: wgpu::Buffer,
    globals_bind_group: Option<wgpu::BindGroup>,
    uploaded_size: Option<[u32; 2]>,
    /// GPU allocations this target could not avoid this frame.
    allocations: usize,
}

pub(super) struct TextGpu {
    pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
}

impl TextGpu {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas: &GlyphAtlasManager,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.text.globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Globals>() as u64
                        ),
                    },
                    count: None,
                },
                storage_entry(1, std::mem::size_of::<TextRunGpu>() as u64),
                storage_entry(2, std::mem::size_of::<TextPresentationGpu>() as u64),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.text.pipeline"),
            bind_group_layouts: &[Some(&globals_layout), Some(atlas.layout())],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.text.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(TEXT_SHADER)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.text.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
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
                module: &shader,
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
        Self {
            pipeline,
            globals_layout,
        }
    }

    pub(super) fn new_target(&self, device: &wgpu::Device) -> TextTargetGpu {
        TextTargetGpu::new(device, &self.globals_layout)
    }

    /// Write this frame's projection, arena blocks and presentation tables.
    ///
    /// Only what changed: the arena is written through the coalesced ranges
    /// [`super::TextPipeline::flush_runs`] staged, and the two tables through a
    /// block diff against what this target already holds. A repaint of
    /// unchanged text costs no queue traffic at all.
    pub(super) fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &mut TextTargetGpu,
        physical_size: [u32; 2],
        frame: &FrameUpload<'_>,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) -> TextUploadBytes {
        let mut rebind = false;
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
        let mut bytes = TextUploadBytes::default();
        if frame.arena_capacity > target.instance_capacity as u32 {
            target.instance_capacity = frame.arena_capacity as usize;
            target.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.instances"),
                size: (target.instance_capacity * std::mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            target.allocations += 1;
        }
        for write in frame.writes {
            let staged = &frame.staging[write.staged.start as usize..write.staged.end as usize];
            let data: &[u8] = bytemuck::cast_slice(staged);
            queue.write_buffer(
                &target.instances,
                write.offset as u64 * std::mem::size_of::<GlyphInstance>() as u64,
                data,
            );
            bytes.instances += data.len();
        }
        if !frame.runs.is_empty() {
            let mut dirty = frame.run_dirty.clone();
            if frame.runs.len() > target.run_capacity {
                target.run_capacity = frame.runs.len().next_power_of_two();
                target.runs = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.text.runs"),
                    size: (target.run_capacity * std::mem::size_of::<TextRunGpu>()) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                target.allocations += 1;
                // The new buffer holds nothing, so every row is owed.
                dirty = Some(0..frame.runs.len() as u32);
                rebind = true;
            }
            if let Some(dirty) = dirty {
                let end = (dirty.end as usize).min(frame.runs.len());
                let start = (dirty.start as usize).min(end);
                let data: &[u8] = bytemuck::cast_slice(&frame.runs[start..end]);
                if !data.is_empty() {
                    queue.write_buffer(
                        &target.runs,
                        (start * std::mem::size_of::<TextRunGpu>()) as u64,
                        data,
                    );
                    bytes.presentation += data.len();
                }
            }
        }
        if !frame.presentations.is_empty() {
            let mut previous: &[TextPresentationGpu] = frame.uploaded_presentations;
            if frame.presentations.len() > target.presentation_capacity {
                target.presentation_capacity = frame.presentations.len().next_power_of_two();
                target.presentations = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.text.presentations"),
                    size: (target.presentation_capacity
                        * std::mem::size_of::<TextPresentationGpu>())
                        as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                target.allocations += 1;
                previous = &[];
                rebind = true;
            }
            bytes.presentation += crate::scene_paint::buffer_upload::upload_changed(
                queue,
                &target.presentations,
                bytemuck::cast_slice(previous),
                bytemuck::cast_slice(frame.presentations),
            );
        }
        if rebind {
            target.globals_bind_group = Some(target.bind(device, &self.globals_layout));
        }
        if let Some(work) = work {
            work.record_upload(bytes.instances + bytes.presentation);
        }
        bytes
    }

    pub(super) fn draw_segment(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target: &TextTargetGpu,
        segment: &DrawSegment,
        bind_group: &wgpu::BindGroup,
    ) {
        if segment.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        let Some(globals) = target.globals_bind_group.as_ref() else {
            return;
        };
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, target.instances.slice(..));
        pass.draw(0..4, segment.first..segment.first + segment.count);
    }
}

/// What one frame hands the GPU. Grouped because the three arrays travel
/// together and their previous contents are the only thing that decides
/// whether a byte moves at all.
pub(super) struct FrameUpload<'a> {
    /// Instance slots the arena must hold; a larger one replaces the buffer.
    pub arena_capacity: u32,
    pub writes: &'a [ArenaWrite],
    pub staging: &'a [GlyphInstance],
    pub runs: &'a [TextRunGpu],
    /// Rows that changed, as one span.
    pub run_dirty: Option<Range<u32>>,
    pub presentations: &'a [TextPresentationGpu],
    pub uploaded_presentations: &'a [TextPresentationGpu],
}

/// One coalesced run of arena slots to write, staged contiguously.
#[derive(Clone, Debug)]
pub(super) struct ArenaWrite {
    pub offset: u32,
    pub staged: Range<u32>,
}

/// Bytes written, split the way [`super::TextGlyphCounters`] reports them:
/// glyph geometry against the presentation tables that only say where it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TextUploadBytes {
    pub instances: usize,
    pub presentation: usize,
}

fn storage_entry(binding: u32, min: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(min),
        },
        count: None,
    }
}

impl TextTargetGpu {
    pub(super) fn take_allocations(&mut self) -> usize {
        std::mem::take(&mut self.allocations)
    }

    fn new(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout) -> Self {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.text.globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut target = Self {
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.instances"),
                size: (INITIAL_INSTANCES * std::mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            instance_capacity: INITIAL_INSTANCES,
            runs: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.runs"),
                size: (INITIAL_RUNS * std::mem::size_of::<TextRunGpu>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            run_capacity: INITIAL_RUNS,
            presentations: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.text.presentations"),
                size: (INITIAL_PRESENTATIONS * std::mem::size_of::<TextPresentationGpu>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            presentation_capacity: INITIAL_PRESENTATIONS,
            // Replaced below, once the buffers the real layout needs exist.
            globals_bind_group: None,
            globals,
            uploaded_size: None,
            allocations: 0,
        };
        target.globals_bind_group = Some(target.bind(device, globals_layout));
        target
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.text.globals.bind"),
            layout: globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.runs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.presentations.as_entire_binding(),
                },
            ],
        })
    }
}

impl GlyphInstance {
    pub(super) fn new(
        origin: [i32; 2],
        size: [u32; 2],
        texel: [u32; 2],
        color: u32,
        control: u32,
    ) -> Self {
        Self {
            origin,
            dim: (size[0] & 0xffff) | ((size[1] & 0xffff) << 16),
            uv: (texel[0] & 0xffff) | ((texel[1] & 0xffff) << 16),
            color,
            control,
        }
    }

    /// Rebind this glyph to `run` without touching the geometry it resolved
    /// to. What a retained entry does when it is drawn a second time under a
    /// different presentation.
    pub(super) fn with_run(self, run: u32) -> Self {
        Self {
            control: (self.control & ((1 << INSTANCE_RUN_SHIFT) - 1)) | (run << INSTANCE_RUN_SHIFT),
            ..self
        }
    }

    /// Re-point this glyph at the rectangle its atlas handle now names.
    pub(super) fn with_placement(self, origin: [u32; 2], size: [u32; 2]) -> Self {
        Self {
            dim: (size[0] & 0xffff) | ((size[1] & 0xffff) << 16),
            uv: (origin[0] & 0xffff) | ((origin[1] & 0xffff) << 16),
            ..self
        }
    }

    /// Keep the slot but cover nothing: the placement this glyph named is gone
    /// and sampling whatever now owns it would draw a different letter.
    pub(super) fn vacated(self) -> Self {
        Self { dim: 0, ..self }
    }

    /// A glyph that covers nothing. Fills the slack a size class leaves after
    /// an entry's glyphs so the arena stays contiguous and neighbouring
    /// entries still batch into one draw.
    pub(super) const VACANT: Self = Self {
        origin: [0, 0],
        dim: 0,
        uv: 0,
        color: 0,
        control: 0,
    };
}

impl TextRunGpu {
    /// A row nothing names: a slot whose entry has retired, or one the table
    /// grew past. Fully transparent, so a stray instance could only draw
    /// nothing.
    pub(super) const VACANT: Self = Self {
        origin: [0.0; 2],
        presentation: 0,
        flags: 0,
        color: [0.0; 4],
        opacity: 0.0,
        pad: [0.0; 3],
    };

    pub(super) fn new(
        origin: [f32; 2],
        presentation: u32,
        flags: u32,
        color: [f32; 4],
        opacity: f32,
    ) -> Self {
        Self {
            origin,
            presentation,
            flags,
            color,
            opacity,
            pad: [0.0; 3],
        }
    }
}

impl TextPresentationGpu {
    pub(super) fn new(
        affine: [f32; 6],
        persp: [f32; 2],
        clip: &clip::FragmentClip,
        scale: f32,
    ) -> Self {
        let mut polygon = [[0.0f32; 4]; 4];
        for (index, slot) in polygon.iter_mut().enumerate() {
            let [ax, ay] = clip.polygon[index * 2];
            let [bx, by] = clip.polygon[index * 2 + 1];
            *slot = [ax, ay, bx, by];
        }
        Self {
            affine: [affine[0], affine[1], affine[2], affine[3]],
            project: [affine[4], affine[5], persp[0], persp[1]],
            clip_rect: clip.rect,
            clip_inv_abcd: clip.inv_abcd,
            clip_inv_ef: [clip.inv_ef[0], clip.inv_ef[1], clip.corner_radius, scale],
            polygon,
            polygon_count: u32::from(clip.polygon_count),
            pad: [0; 3],
        }
    }

    /// Bit pattern, so two runs that present identically share one row.
    pub(super) fn to_bits(self) -> [u32; 40] {
        let mut bits = [0u32; 40];
        let floats: [f32; 36] = bytemuck::cast([
            self.affine,
            self.project,
            self.clip_rect,
            self.clip_inv_abcd,
            self.clip_inv_ef,
            self.polygon[0],
            self.polygon[1],
            self.polygon[2],
            self.polygon[3],
        ]);
        for (slot, value) in bits.iter_mut().zip(floats.iter()) {
            *slot = value.to_bits();
        }
        bits[36] = self.polygon_count;
        bits
    }
}

/// sRGB `a<<24 | r<<16 | g<<8 | b`, the form the instance and the retained
/// placement both carry.
pub(super) fn pack_srgb(color: [f32; 4]) -> u32 {
    let [r, g, b, a] = crate::scene_paint::color::to_rgba8(color);
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}
