use bytemuck::{Pod, Zeroable};
use lyon::math::{Box2D, Point, point};
use lyon::path::{Path, Winding, builder::BorderRadii};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use nana_ui_scene::{IconPathCommand, IconShape, icon_geometry};

use super::clip::LogicalRect;
use super::color::{orthographic_scaled, pack_linear, with_opacity};
use crate::PhysicalRect;
use crate::icons::Icon;

const INITIAL_VERTICES: usize = 1_024;
const INITIAL_INDICES: usize = 2_048;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 2],
    color: [f32; 4],
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.triangle.solid.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("solid_vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Float32x4,
                    ),
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
                count: 4,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
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
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_vertices.clear();
        self.pending_indices.clear();
    }

    pub(super) fn push_icon(
        &mut self,
        bounds: LogicalRect,
        icon: Icon,
        color: [f32; 4],
        opacity: f32,
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
        let start = self.pending_indices.len() as u32;
        tessellate_icon(
            &mut self.fill,
            &mut self.stroke,
            &mut self.pending_vertices,
            &mut self.pending_indices,
            icon,
            scale,
            offset,
            color,
            1.7,
        );
        let index_count = self.pending_indices.len() as u32 - start;
        (index_count > 0).then_some(MeshRange {
            first_index: start,
            index_count,
        })
    }

    pub(super) fn push_stroke(
        &mut self,
        points: &[[f32; 2]],
        width: f32,
        color: [f32; 4],
        opacity: f32,
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
        stroke_path(
            &mut self.stroke,
            &mut self.pending_vertices,
            &mut self.pending_indices,
            &builder.build(),
            width,
            pack_linear(with_opacity(color, opacity)),
        );
        let index_count = self.pending_indices.len() as u32 - start;
        (index_count > 0).then_some(MeshRange {
            first_index: start,
            index_count,
        })
    }

    pub(super) fn push_spinner(
        &mut self,
        bounds: LogicalRect,
        phase: u8,
        color: [f32; 4],
        opacity: f32,
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
    ) {
        let uniforms = Uniforms {
            transform: orthographic_scaled(physical_size[0], physical_size[1], scale_factor),
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        if !self.pending_vertices.is_empty() {
            if self.pending_vertices.len() > self.vertex_capacity {
                self.vertex_capacity = self.pending_vertices.len().next_power_of_two();
                self.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nana-ui.scene.triangle.vertices"),
                    size: (self.vertex_capacity * std::mem::size_of::<MeshVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(
                &self.vertices,
                0,
                bytemuck::cast_slice(&self.pending_vertices),
            );
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
            }
            queue.write_buffer(
                &self.indices,
                0,
                bytemuck::cast_slice(&self.pending_indices),
            );
        }
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: &MeshRange,
        scissor: PhysicalRect,
    ) {
        if range.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw_indexed(
            range.first_index..range.first_index + range.index_count,
            0,
            0..1,
        );
    }
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
        match shape {
            IconShape::Path(commands) => {
                let mut builder = Path::builder();
                let mut open = false;
                for command in commands {
                    match command {
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

fn append_geometry(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    geometry: &VertexBuffers<Point, u32>,
    color: [f32; 4],
) {
    let base = vertices.len() as u32;
    vertices.extend(geometry.vertices.iter().map(|position| MeshVertex {
        position: [position.x, position.y],
        color,
    }));
    indices.extend(geometry.indices.iter().map(|index| base + *index));
}
