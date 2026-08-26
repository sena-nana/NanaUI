use std::collections::{HashMap, VecDeque};

use bytemuck::{Pod, Zeroable};
use lyon::{
    math::{Box2D, Point, point},
    path::{Path, Winding, builder::BorderRadii},
    tessellation::{
        BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
        StrokeVertex, VertexBuffers,
    },
};
use nana_ui_scene::{IconPathCommand, IconShape, icon_geometry};

use super::{
    clip::{FragmentClip, LogicalRect},
    color::{orthographic_scaled, pack_linear, with_opacity},
};
use crate::{PhysicalRect, icons::Icon};

const INITIAL_VERTICES: usize = 1_024;
const INITIAL_INDICES: usize = 2_048;
const ICON_CACHE_CAP: usize = 256;

/// Tessellated icon geometry reuse: icons redraw identically every frame at
/// the same position/size/color, and lyon tessellation is the icon-path CPU
/// cost. Indices are stored relative to the icon's vertex base.
#[derive(Default)]
struct IconCache {
    entries: HashMap<IconKey, CachedIcon>,
    order: VecDeque<IconKey>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct IconKey {
    icon: usize,
    scale_bits: u32,
    offset_bits: [u32; 2],
    color_bits: [u32; 4],
}

struct CachedIcon {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

impl IconCache {
    fn insert(&mut self, key: IconKey, cached: CachedIcon) {
        if self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= ICON_CACHE_CAP {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, cached);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 2],
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 2],
}

impl MeshVertex {
    fn new(position: [f32; 2], color: [f32; 4]) -> Self {
        Self {
            position,
            color,
            clip_rect: FragmentClip::PASS.rect,
            clip_inv_abcd: FragmentClip::PASS.inv_abcd,
            clip_inv_ef: FragmentClip::PASS.inv_ef,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
}

pub(super) struct MeshRange {
    pub first_index: u32,
    pub index_count: u32,
}

pub(super) struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    pipeline_msaa: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_capacity: usize,
    index_capacity: usize,
    pending_vertices: Vec<MeshVertex>,
    pending_indices: Vec<u32>,
    fill: FillTessellator,
    stroke: StrokeTessellator,
    icon_cache: IconCache,
}

impl MeshPipeline {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.triangle.solid.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("shader/triangle.wgsl"),
                "\n",
                include_str!("shader/triangle_solid.wgsl"),
                "\n",
                include_str!("shader/color.wgsl"),
            ))),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.triangle.uniforms"),
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
            label: Some("nana-ui.scene.triangle.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.triangle.uniforms.bind_group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.triangle.solid.pipeline"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = mesh_pipeline(device, &shader, &layout, format, 1);
        let pipeline_msaa = mesh_pipeline(device, &shader, &layout, format, 4);
        Self {
            pipeline,
            pipeline_msaa,
            bind_group,
            uniforms,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.triangle.vertices"),
                size: (INITIAL_VERTICES * std::mem::size_of::<MeshVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            indices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.triangle.indices"),
                size: (INITIAL_INDICES * std::mem::size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_capacity: INITIAL_VERTICES,
            index_capacity: INITIAL_INDICES,
            pending_vertices: Vec::new(),
            pending_indices: Vec::new(),
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
            icon_cache: IconCache::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_vertices.clear();
        self.pending_indices.clear();
    }

    pub(super) fn push_icon(
        &mut self,
        bounds: LogicalRect,
        affine: [f32; 6],
        icon: Icon,
        color: [f32; 4],
        opacity: f32,
        fragment_clip: FragmentClip,
    ) -> Option<MeshRange> {
        let scale = bounds.width.min(bounds.height) / 24.0;
        if scale <= 0.0 {
            return None;
        }
        let offset = [
            bounds.x + (bounds.width - 24.0 * scale) / 2.0,
            bounds.y + (bounds.height - 24.0 * scale) / 2.0,
        ];
        let color = pack_linear(with_opacity(color, opacity));
        let key = IconKey {
            icon: icon.as_ptr() as usize,
            scale_bits: scale.to_bits(),
            offset_bits: [offset[0].to_bits(), offset[1].to_bits()],
            color_bits: color.map(f32::to_bits),
        };
        let start = self.pending_indices.len() as u32;
        let vertex_base = self.pending_vertices.len();
        if let Some(cached) = self.icon_cache.entries.get(&key) {
            let base = vertex_base as u32;
            self.pending_vertices.extend_from_slice(&cached.vertices);
            self.pending_indices
                .extend(cached.indices.iter().map(|index| index + base));
        } else {
            tessellate_icon(
                &mut self.fill,
                &mut self.stroke,
                &mut self.pending_vertices,
                &mut self.pending_indices,
                icon,
                scale,
                offset,
                color,
                2.0,
            );
            self.icon_cache.insert(
                key,
                CachedIcon {
                    vertices: self.pending_vertices[vertex_base..].to_vec(),
                    indices: self.pending_indices[start as usize..]
                        .iter()
                        .map(|index| index - vertex_base as u32)
                        .collect(),
                },
            );
        }
        let index_count = self.pending_indices.len() as u32 - start;
        apply_affine_to_vertices(&mut self.pending_vertices, vertex_base, affine);
        stamp_fragment_clip(&mut self.pending_vertices, vertex_base, fragment_clip);
        (index_count > 0).then_some(MeshRange {
            first_index: start,
            index_count,
        })
    }

    pub(super) fn push_stroke(
        &mut self,
        points: &[[f32; 2]],
        affine: [f32; 6],
        width: f32,
        color: [f32; 4],
        opacity: f32,
        fragment_clip: FragmentClip,
    ) -> Option<MeshRange> {
        if points.len() < 2 || width <= 0.0 {
            return None;
        }
        let mut builder = Path::builder();
        builder.begin(point(points[0][0], points[0][1]));
        for sample in points.iter().skip(1) {
            builder.line_to(point(sample[0], sample[1]));
        }
        builder.end(false);
        let start = self.pending_indices.len() as u32;
        let vertex_base = self.pending_vertices.len();
        stroke_path(
            &mut self.stroke,
            &mut self.pending_vertices,
            &mut self.pending_indices,
            &builder.build(),
            width,
            pack_linear(with_opacity(color, opacity)),
        );
        apply_affine_to_vertices(&mut self.pending_vertices, vertex_base, affine);
        stamp_fragment_clip(&mut self.pending_vertices, vertex_base, fragment_clip);
        let index_count = self.pending_indices.len() as u32 - start;
        (index_count > 0).then_some(MeshRange {
            first_index: start,
            index_count,
        })
    }

    pub(super) fn push_spinner(
        &mut self,
        bounds: LogicalRect,
        affine: [f32; 6],
        phase: u8,
        color: [f32; 4],
        opacity: f32,
        fragment_clip: FragmentClip,
    ) -> Option<MeshRange> {
        let scale = bounds.width.min(bounds.height) / 24.0;
        if scale <= 0.0 {
            return None;
        }
        let color = with_opacity(color, opacity);
        let center = [
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ];
        let start = self.pending_indices.len() as u32;
        let vertex_base = self.pending_vertices.len();
        for index in 0..8_u8 {
            let angle = f32::from(index) * std::f32::consts::FRAC_PI_4;
            let from = point(
                center[0] + angle.cos() * 6.0 * scale,
                center[1] + angle.sin() * 6.0 * scale,
            );
            let to = point(
                center[0] + angle.cos() * 10.0 * scale,
                center[1] + angle.sin() * 10.0 * scale,
            );
            let distance = (index + 8 - phase % 8) % 8;
            let alpha = 1.0 - f32::from(distance) * 0.105;
            let mut tick_color = color;
            tick_color[3] *= alpha;
            let mut path = Path::builder();
            path.begin(from);
            path.line_to(to);
            path.end(false);
            stroke_path(
                &mut self.stroke,
                &mut self.pending_vertices,
                &mut self.pending_indices,
                &path.build(),
                2.2 * scale,
                pack_linear(tick_color),
            );
        }
        apply_affine_to_vertices(&mut self.pending_vertices, vertex_base, affine);
        stamp_fragment_clip(&mut self.pending_vertices, vertex_base, fragment_clip);
        let index_count = self.pending_indices.len() as u32 - start;
        (index_count > 0).then_some(MeshRange {
            first_index: start,
            index_count,
        })
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
            transform: orthographic_scaled(physical_size[0], physical_size[1], scale_factor),
        };
        let uniform_bytes = bytemuck::bytes_of(&uniforms);
        queue.write_buffer(&self.uniforms, 0, uniform_bytes);
        if let Some(work) = gpu_work {
            work.record_upload(uniform_bytes.len());
        }
        if !self.pending_vertices.is_empty() {
            if self.pending_vertices.len() > self.vertex_capacity {
                self.vertex_capacity = self.pending_vertices.len().next_power_of_two();
                self.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.triangle.vertices"),
                    size: (self.vertex_capacity * std::mem::size_of::<MeshVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if let Some(work) = gpu_work {
                    work.record_realloc();
                }
            }
            let vertex_bytes = bytemuck::cast_slice(&self.pending_vertices);
            queue.write_buffer(&self.vertices, 0, vertex_bytes);
            if let Some(work) = gpu_work {
                work.record_upload(vertex_bytes.len());
                work.record_batch_rebuild();
            }
        }
        if !self.pending_indices.is_empty() {
            if self.pending_indices.len() > self.index_capacity {
                self.index_capacity = self.pending_indices.len().next_power_of_two();
                self.indices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.triangle.indices"),
                    size: (self.index_capacity * std::mem::size_of::<u32>()) as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if let Some(work) = gpu_work {
                    work.record_realloc();
                }
            }
            let index_bytes = bytemuck::cast_slice(&self.pending_indices);
            queue.write_buffer(&self.indices, 0, index_bytes);
            if let Some(work) = gpu_work {
                work.record_upload(index_bytes.len());
            }
        }
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: &MeshRange,
        scissor: PhysicalRect,
        sample_count: u32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        if range.index_count == 0 {
            return;
        }
        pass.set_pipeline(if sample_count > 1 {
            &self.pipeline_msaa
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw_indexed(
            range.first_index..range.first_index + range.index_count,
            0,
            0..1,
        );
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}

fn mesh_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("nana-ui.scene.triangle.solid.pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("solid_vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<MeshVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array!(
                    0 => Float32x2,
                    1 => Float32x4,
                    2 => Float32x4,
                    3 => Float32x4,
                    4 => Float32x2,
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

#[allow(clippy::too_many_arguments)]
fn tessellate_icon(
    fill: &mut FillTessellator,
    stroke: &mut StrokeTessellator,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    icon: Icon,
    scale: f32,
    offset: [f32; 2],
    color: [f32; 4],
    stroke_width: f32,
) {
    let map = |x: f32, y: f32| point(offset[0] + x * scale, offset[1] + y * scale);
    for shape in icon_geometry(icon).shapes {
        match *shape {
            IconShape::Path(commands) => {
                let mut builder = Path::builder();
                let mut open = false;
                for command in commands {
                    match *command {
                        IconPathCommand::MoveTo([x, y]) => {
                            if open {
                                builder.end(false);
                            }
                            builder.begin(map(x, y));
                            open = true;
                        }
                        IconPathCommand::LineTo([x, y]) => {
                            if !open {
                                builder.begin(map(x, y));
                                open = true;
                            } else {
                                builder.line_to(map(x, y));
                            }
                        }
                        IconPathCommand::CubicTo {
                            control_a: [ax, ay],
                            control_b: [bx, by],
                            to: [x, y],
                        } => {
                            if open {
                                builder.cubic_bezier_to(map(ax, ay), map(bx, by), map(x, y));
                            }
                        }
                        IconPathCommand::Close => {
                            if open {
                                builder.close();
                                open = false;
                            }
                        }
                    }
                }
                if open {
                    builder.end(false);
                }
                stroke_path(
                    stroke,
                    vertices,
                    indices,
                    &builder.build(),
                    stroke_width * scale,
                    color,
                );
            }
            IconShape::Circle {
                center: [x, y],
                radius,
            } => {
                let mut builder = Path::builder();
                builder.add_circle(map(x, y), radius * scale, Winding::Positive);
                stroke_path(
                    stroke,
                    vertices,
                    indices,
                    &builder.build(),
                    stroke_width * scale,
                    color,
                );
            }
            IconShape::Rect {
                origin: [x, y],
                size: [width, height],
                filled,
            } => {
                let rect = Box2D {
                    min: map(x, y),
                    max: map(x + width, y + height),
                };
                let mut builder = Path::builder();
                builder.add_rectangle(&rect, Winding::Positive);
                let path = builder.build();
                if filled {
                    fill_path(fill, vertices, indices, &path, color);
                } else {
                    stroke_path(
                        stroke,
                        vertices,
                        indices,
                        &path,
                        stroke_width * scale,
                        color,
                    );
                }
            }
            IconShape::RoundedRect {
                origin: [x, y],
                size: [width, height],
                radius,
            } => {
                let mut builder = Path::builder();
                builder.add_rounded_rectangle(
                    &Box2D {
                        min: map(x, y),
                        max: map(x + width, y + height),
                    },
                    &BorderRadii {
                        top_left: radius * scale,
                        top_right: radius * scale,
                        bottom_left: radius * scale,
                        bottom_right: radius * scale,
                    },
                    Winding::Positive,
                );
                stroke_path(
                    stroke,
                    vertices,
                    indices,
                    &builder.build(),
                    stroke_width * scale,
                    color,
                );
            }
        }
    }
}

fn stroke_path(
    tessellator: &mut StrokeTessellator,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    path: &Path,
    width: f32,
    color: [f32; 4],
) {
    let mut geometry: VertexBuffers<Point, u32> = VertexBuffers::new();
    let options = StrokeOptions::tolerance(0.1)
        .with_line_width(width)
        .with_line_cap(lyon::tessellation::LineCap::Round)
        .with_line_join(lyon::tessellation::LineJoin::Round);
    let _ = tessellator.tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| vertex.position()),
    );
    append_geometry(vertices, indices, &geometry, color);
}

fn fill_path(
    tessellator: &mut FillTessellator,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    path: &Path,
    color: [f32; 4],
) {
    let mut geometry: VertexBuffers<Point, u32> = VertexBuffers::new();
    let _ = tessellator.tessellate_path(
        path,
        &FillOptions::tolerance(0.15),
        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| vertex.position()),
    );
    append_geometry(vertices, indices, &geometry, color);
}

fn apply_affine_to_vertices(vertices: &mut [MeshVertex], start: usize, affine: [f32; 6]) {
    if affine == super::clip::IDENTITY_AFFINE {
        return;
    }
    for vertex in &mut vertices[start..] {
        vertex.position =
            super::clip::transform_point(affine, vertex.position[0], vertex.position[1]);
    }
}

fn stamp_fragment_clip(vertices: &mut [MeshVertex], start: usize, clip: FragmentClip) {
    for vertex in &mut vertices[start..] {
        vertex.clip_rect = clip.rect;
        vertex.clip_inv_abcd = clip.inv_abcd;
        vertex.clip_inv_ef = clip.inv_ef;
    }
}

fn append_geometry(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    geometry: &VertexBuffers<Point, u32>,
    color: [f32; 4],
) {
    let base = vertices.len() as u32;
    vertices.extend(
        geometry
            .vertices
            .iter()
            .map(|position| MeshVertex::new([position.x, position.y], color)),
    );
    indices.extend(geometry.indices.iter().map(|index| base + *index));
}
