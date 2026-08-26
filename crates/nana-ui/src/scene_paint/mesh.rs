use bytemuck::{Pod, Zeroable};

use super::{
    clip::{FragmentClip, LogicalRect},
    color::{orthographic_scaled, pack_linear, with_opacity},
};
use crate::PhysicalRect;

const INITIAL_VERTICES: usize = 1_024;
const INITIAL_INDICES: usize = 2_048;
/// Extra logical pixels so the covering quad has room for `fwidth` AA.
const STROKE_AA_FRINGE: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 2],
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 3],
    /// Segment start and constant radius (`r0 = r1 = width / 2`).
    p0_radius: [f32; 3],
    p1: [f32; 2],
}

impl MeshVertex {
    fn capsule(
        position: [f32; 2],
        color: [f32; 4],
        p0: [f32; 2],
        p1: [f32; 2],
        radius: f32,
    ) -> Self {
        Self {
            position,
            color,
            clip_rect: FragmentClip::PASS.rect,
            clip_inv_abcd: FragmentClip::PASS.inv_abcd,
            clip_inv_ef: [
                FragmentClip::PASS.inv_ef[0],
                FragmentClip::PASS.inv_ef[1],
                FragmentClip::PASS.corner_radius,
            ],
            p0_radius: [p0[0], p0[1], radius],
            p1,
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
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_vertices.clear();
        self.pending_indices.clear();
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
        let start = self.pending_indices.len() as u32;
        let vertex_base = self.pending_vertices.len();
        append_stroke_geometry(
            &mut self.pending_vertices,
            &mut self.pending_indices,
            points,
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
        let radius = 2.2 * scale * 0.5;
        for index in 0..8_u8 {
            let angle = f32::from(index) * std::f32::consts::FRAC_PI_4;
            let from = [
                center[0] + angle.cos() * 6.0 * scale,
                center[1] + angle.sin() * 6.0 * scale,
            ];
            let to = [
                center[0] + angle.cos() * 10.0 * scale,
                center[1] + angle.sin() * 10.0 * scale,
            ];
            let distance = (index + 8 - phase % 8) % 8;
            let alpha = 1.0 - f32::from(distance) * 0.105;
            let mut tick_color = color;
            tick_color[3] *= alpha;
            push_capsule_segment(
                &mut self.pending_vertices,
                &mut self.pending_indices,
                from,
                to,
                radius,
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
                    4 => Float32x3,
                    5 => Float32x3,
                    6 => Float32x2,
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

fn append_stroke_geometry(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    points: &[[f32; 2]],
    width: f32,
    color: [f32; 4],
) {
    let radius = width * 0.5;
    for pair in points.windows(2) {
        push_capsule_segment(vertices, indices, pair[0], pair[1], radius, color);
    }
}

/// One articulated-line segment: a rectangle covering the constant-radius capsule.
///
/// Adjacent segments share an endpoint disc, so joins fill without extra miters.
fn push_capsule_segment(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 2],
    p1: [f32; 2],
    radius: f32,
    color: [f32; 4],
) {
    let Some(corners) = covering_quad(p0, p1, radius) else {
        return;
    };
    let base = vertices.len() as u32;
    for corner in corners {
        vertices.push(MeshVertex::capsule(corner, color, p0, p1, radius));
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn covering_quad(p0: [f32; 2], p1: [f32; 2], radius: f32) -> Option<[[f32; 2]; 4]> {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let length = dx.hypot(dy);
    if !length.is_finite() || length < f32::EPSILON || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let tx = dx / length;
    let ty = dy / length;
    let nx = -ty;
    let ny = tx;
    let pad = radius + STROKE_AA_FRINGE;
    let ax = p0[0] - tx * pad;
    let ay = p0[1] - ty * pad;
    let bx = p1[0] + tx * pad;
    let by = p1[1] + ty * pad;
    Some([
        [ax - nx * pad, ay - ny * pad],
        [ax + nx * pad, ay + ny * pad],
        [bx + nx * pad, by + ny * pad],
        [bx - nx * pad, by - ny * pad],
    ])
}

fn capsule_distance(point: [f32; 2], p0: [f32; 2], p1: [f32; 2]) -> f32 {
    let pa = [point[0] - p0[0], point[1] - p0[1]];
    let ba = [p1[0] - p0[0], p1[1] - p0[1]];
    let denom = ba[0] * ba[0] + ba[1] * ba[1];
    let t = if denom <= f32::EPSILON {
        0.0
    } else {
        ((pa[0] * ba[0] + pa[1] * ba[1]) / denom).clamp(0.0, 1.0)
    };
    let closest = [p0[0] + ba[0] * t, p0[1] + ba[1] * t];
    (point[0] - closest[0]).hypot(point[1] - closest[1])
}

fn apply_affine_to_vertices(vertices: &mut [MeshVertex], start: usize, affine: [f32; 6]) {
    if affine == super::clip::IDENTITY_AFFINE {
        return;
    }
    let [a, b, c, d, _, _] = affine;
    let scale = ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) * 0.5;
    for vertex in &mut vertices[start..] {
        vertex.position =
            super::clip::transform_point(affine, vertex.position[0], vertex.position[1]);
        let p0 = super::clip::transform_point(affine, vertex.p0_radius[0], vertex.p0_radius[1]);
        vertex.p0_radius[0] = p0[0];
        vertex.p0_radius[1] = p0[1];
        vertex.p0_radius[2] *= scale;
        vertex.p1 = super::clip::transform_point(affine, vertex.p1[0], vertex.p1[1]);
    }
}

fn stamp_fragment_clip(vertices: &mut [MeshVertex], start: usize, clip: FragmentClip) {
    for vertex in &mut vertices[start..] {
        vertex.clip_rect = clip.rect;
        vertex.clip_inv_abcd = clip.inv_abcd;
        vertex.clip_inv_ef = [clip.inv_ef[0], clip.inv_ef[1], clip.corner_radius];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_segment_emits_four_covering_vertices() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_stroke_geometry(
            &mut vertices,
            &mut indices,
            &[[0.0, 0.0], [10.0, 0.0]],
            4.0,
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(vertices.len(), 4);
        assert_eq!(indices, [0, 1, 2, 0, 2, 3]);
        for vertex in &vertices {
            assert_eq!(vertex.p0_radius, [0.0, 0.0, 2.0]);
            assert_eq!(vertex.p1, [10.0, 0.0]);
        }
        let xs: Vec<f32> = vertices.iter().map(|vertex| vertex.position[0]).collect();
        let ys: Vec<f32> = vertices.iter().map(|vertex| vertex.position[1]).collect();
        let pad = 2.0 + STROKE_AA_FRINGE;
        assert!(xs.iter().copied().fold(f32::MAX, f32::min) <= -pad + 1e-4);
        assert!(xs.iter().copied().fold(f32::MIN, f32::max) >= 10.0 + pad - 1e-4);
        assert!(ys.iter().copied().fold(f32::MAX, f32::min) <= -pad + 1e-4);
        assert!(ys.iter().copied().fold(f32::MIN, f32::max) >= pad - 1e-4);
    }

    #[test]
    fn zero_length_and_empty_strokes_emit_nothing() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_stroke_geometry(
            &mut vertices,
            &mut indices,
            &[[3.0, 3.0], [3.0, 3.0]],
            2.0,
            [1.0, 1.0, 1.0, 1.0],
        );
        assert!(vertices.is_empty());
        assert!(indices.is_empty());
        append_stroke_geometry(&mut vertices, &mut indices, &[[1.0, 1.0]], 2.0, [1.0; 4]);
        assert!(vertices.is_empty());
    }

    #[test]
    fn two_segments_share_an_endpoint_disc() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_stroke_geometry(
            &mut vertices,
            &mut indices,
            &[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0]],
            2.0,
            [0.0, 0.0, 1.0, 1.0],
        );
        assert_eq!(vertices.len(), 8);
        assert_eq!(indices.len(), 12);
        let join = [8.0, 0.0];
        let first = capsule_distance(
            join,
            vertices[0].p0_radius[..2].try_into().unwrap(),
            vertices[0].p1,
        );
        let second = capsule_distance(
            join,
            vertices[4].p0_radius[..2].try_into().unwrap(),
            vertices[4].p1,
        );
        assert!(first <= vertices[0].p0_radius[2] + 1e-5);
        assert!(second <= vertices[4].p0_radius[2] + 1e-5);
    }

    #[test]
    fn capsule_covers_midline_and_rejects_far_normal() {
        let p0 = [0.0, 0.0];
        let p1 = [20.0, 0.0];
        let radius = 1.5;
        assert!(capsule_distance([10.0, 0.0], p0, p1) < radius);
        assert!(capsule_distance([0.0, 0.0], p0, p1) < radius);
        assert!(capsule_distance([-0.5, 0.0], p0, p1) < radius);
        assert!(capsule_distance([10.0, 4.0], p0, p1) > radius);
        let corners = covering_quad(p0, p1, radius).expect("quad");
        let outside_corner = corners[0];
        assert!(
            capsule_distance(outside_corner, p0, p1) > radius,
            "covering-quad corner must be discarded by the capsule, got {}",
            capsule_distance(outside_corner, p0, p1)
        );
    }

    #[test]
    fn affine_scales_radius_and_moves_endpoints() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_stroke_geometry(
            &mut vertices,
            &mut indices,
            &[[0.0, 0.0], [4.0, 0.0]],
            2.0,
            [1.0, 1.0, 1.0, 1.0],
        );
        apply_affine_to_vertices(&mut vertices, 0, [2.0, 0.0, 0.0, 2.0, 5.0, 7.0]);
        assert!((vertices[0].p0_radius[0] - 5.0).abs() < 1e-5);
        assert!((vertices[0].p0_radius[1] - 7.0).abs() < 1e-5);
        assert!((vertices[0].p1[0] - 13.0).abs() < 1e-5);
        assert!((vertices[0].p0_radius[2] - 2.0).abs() < 1e-5);
    }
}
