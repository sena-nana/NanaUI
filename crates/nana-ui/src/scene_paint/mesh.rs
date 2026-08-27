use bytemuck::{Pod, Zeroable};
use nana_ui_scene::StrokeCap;

use super::{
    clip::{FragmentClip, LogicalRect},
    color::{orthographic_scaled, pack_linear, with_opacity},
};
use crate::PhysicalRect;

const INITIAL_INSTANCES: usize = 256;
const ROUND_CAP: f32 = 0.0;
const BUTT_CAP: f32 = 1.0;

pub(super) struct StrokeStyle<'a> {
    pub width: f32,
    pub widths: &'a [f32],
    pub cap: StrokeCap,
    pub dash: &'a [f32],
    pub dash_offset: f32,
    pub colors: &'a [[f32; 4]],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct MeshInstance {
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    /// `xyz` = clip inverse e/f + corner radius; `w` is 0 for round, 1 for butt.
    clip_inv_ef_cap: [f32; 4],
    p0: [f32; 2],
    p1: [f32; 2],
    radii: [f32; 2],
}

impl MeshInstance {
    fn segment(
        color: [f32; 4],
        p0: [f32; 2],
        p1: [f32; 2],
        r0: f32,
        r1: f32,
        start_cap: StrokeCap,
        end_cap: StrokeCap,
    ) -> Self {
        Self {
            color,
            clip_rect: FragmentClip::PASS.rect,
            clip_inv_abcd: FragmentClip::PASS.inv_abcd,
            clip_inv_ef_cap: [
                FragmentClip::PASS.inv_ef[0],
                FragmentClip::PASS.inv_ef[1],
                FragmentClip::PASS.corner_radius,
                pack_caps(start_cap, end_cap),
            ],
            p0,
            p1,
            radii: [r0, r1],
        }
    }
}

fn cap_code(cap: StrokeCap) -> f32 {
    match cap {
        StrokeCap::Round => ROUND_CAP,
        StrokeCap::Butt | StrokeCap::Square => BUTT_CAP,
    }
}

fn pack_caps(start: StrokeCap, end: StrokeCap) -> f32 {
    cap_code(start) + 2.0 * cap_code(end)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
}

pub(super) struct MeshRange {
    pub first_instance: u32,
    pub instance_count: u32,
}

pub(super) struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    pipeline_msaa: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    pending_instances: Vec<MeshInstance>,
    uploaded_instances: Vec<MeshInstance>,
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
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.triangle.instances"),
                size: (INITIAL_INSTANCES * std::mem::size_of::<MeshInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            instance_capacity: INITIAL_INSTANCES,
            pending_instances: Vec::new(),
            uploaded_instances: Vec::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_instances.clear();
    }

    pub(super) fn push_stroke(
        &mut self,
        points: &[[f32; 2]],
        style: StrokeStyle<'_>,
        affine: [f32; 6],
        color: [f32; 4],
        opacity: f32,
        fragment_clip: FragmentClip,
    ) -> Option<MeshRange> {
        if points.len() < 2 || style.width <= 0.0 {
            return None;
        }
        let start = self.pending_instances.len() as u32;
        let packed_color = pack_linear(with_opacity(color, opacity));
        let packed_colors: Vec<[f32; 4]> = if style.colors.len() == points.len() {
            style
                .colors
                .iter()
                .map(|item| pack_linear(with_opacity(*item, opacity)))
                .collect()
        } else {
            Vec::new()
        };
        emit_stroke_instances(
            &mut self.pending_instances,
            points,
            StrokeStyle {
                colors: &packed_colors,
                ..style
            },
            packed_color,
        );
        apply_affine_to_instances(&mut self.pending_instances, start as usize, affine);
        stamp_fragment_clip(&mut self.pending_instances, start as usize, fragment_clip);
        let instance_count = self.pending_instances.len() as u32 - start;
        (instance_count > 0).then_some(MeshRange {
            first_instance: start,
            instance_count,
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
        let start = self.pending_instances.len() as u32;
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
            push_segment(
                &mut self.pending_instances,
                from,
                to,
                radius,
                radius,
                StrokeCap::Round,
                StrokeCap::Round,
                pack_linear(tick_color),
            );
        }
        apply_affine_to_instances(&mut self.pending_instances, start as usize, affine);
        stamp_fragment_clip(&mut self.pending_instances, start as usize, fragment_clip);
        let instance_count = self.pending_instances.len() as u32 - start;
        (instance_count > 0).then_some(MeshRange {
            first_instance: start,
            instance_count,
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
        if self.pending_instances.is_empty() {
            self.uploaded_instances.clear();
            return;
        }
        if self.pending_instances == self.uploaded_instances {
            return;
        }
        if self.pending_instances.len() > self.instance_capacity {
            self.instance_capacity = self.pending_instances.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.triangle.instances"),
                size: (self.instance_capacity * std::mem::size_of::<MeshInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(work) = gpu_work {
                work.record_realloc();
            }
        }
        let instance_bytes = bytemuck::cast_slice(&self.pending_instances);
        queue.write_buffer(&self.instances, 0, instance_bytes);
        self.uploaded_instances.clone_from(&self.pending_instances);
        if let Some(work) = gpu_work {
            work.record_upload(instance_bytes.len());
            work.record_batch_rebuild();
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
        if range.instance_count == 0 {
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
        pass.draw(
            0..6,
            range.first_instance..range.first_instance + range.instance_count,
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
                array_stride: std::mem::size_of::<MeshInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array!(
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                    3 => Float32x4,
                    4 => Float32x2,
                    5 => Float32x2,
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

/// Solid uniform-color Round/Butt path used by GraphCanvas and TimeSeries.
/// Decorations (dash, per-point colors, Square) take a separate walk so this
/// loop stays the unused-attribute fast path.
fn emit_stroke_instances(
    instances: &mut Vec<MeshInstance>,
    points: &[[f32; 2]],
    style: StrokeStyle<'_>,
    color: [f32; 4],
) {
    let colors = if style.colors.len() == points.len() {
        style.colors
    } else {
        &[]
    };
    if style.dash.is_empty() && colors.is_empty() && style.cap != StrokeCap::Square {
        append_stroke_instances(
            instances,
            points,
            style.width,
            style.widths,
            style.cap,
            color,
        );
        return;
    }
    match normalize_dash(style.dash) {
        DashPattern::Empty => {}
        DashPattern::Solid => append_styled_stroke_instances(
            instances,
            points,
            style.width,
            style.widths,
            style.cap,
            color,
            colors,
        ),
        DashPattern::Cycle(pattern) => append_dashed_stroke_instances(
            instances,
            points,
            style.width,
            style.widths,
            style.cap,
            color,
            colors,
            &pattern,
            style.dash_offset,
        ),
    }
}

fn append_stroke_instances(
    instances: &mut Vec<MeshInstance>,
    points: &[[f32; 2]],
    width: f32,
    widths: &[f32],
    cap: StrokeCap,
    color: [f32; 4],
) {
    let per_point = widths.len() == points.len();
    let mut segments = Vec::with_capacity(points.len().saturating_sub(1));
    for (index, pair) in points.windows(2).enumerate() {
        let r0 = if per_point {
            widths[index] * 0.5
        } else {
            width * 0.5
        };
        let r1 = if per_point {
            widths[index + 1] * 0.5
        } else {
            width * 0.5
        };
        if segment_is_drawable(pair[0], pair[1], r0, r1) {
            segments.push((pair[0], pair[1], r0.max(0.0), r1.max(0.0)));
        }
    }
    let last = segments.len().saturating_sub(1);
    for (index, (p0, p1, r0, r1)) in segments.into_iter().enumerate() {
        let start_cap = if index == 0 { cap } else { StrokeCap::Butt };
        let end_cap = if index == last || cap == StrokeCap::Butt {
            cap
        } else {
            StrokeCap::Round
        };
        push_segment(instances, p0, p1, r0, r1, start_cap, end_cap, color);
    }
}

fn append_styled_stroke_instances(
    instances: &mut Vec<MeshInstance>,
    points: &[[f32; 2]],
    width: f32,
    widths: &[f32],
    cap: StrokeCap,
    color: [f32; 4],
    colors: &[[f32; 4]],
) {
    let per_point = widths.len() == points.len();
    let per_color = colors.len() == points.len();
    let mut segments = Vec::with_capacity(points.len().saturating_sub(1));
    for (index, pair) in points.windows(2).enumerate() {
        let r0 = if per_point {
            widths[index] * 0.5
        } else {
            width * 0.5
        };
        let r1 = if per_point {
            widths[index + 1] * 0.5
        } else {
            width * 0.5
        };
        if !segment_is_drawable(pair[0], pair[1], r0, r1) {
            continue;
        }
        let segment_color = if per_color { colors[index] } else { color };
        segments.push((pair[0], pair[1], r0.max(0.0), r1.max(0.0), segment_color));
    }
    let last = segments.len().saturating_sub(1);
    for (index, (p0, p1, r0, r1, segment_color)) in segments.into_iter().enumerate() {
        let start_cap = if index == 0 { cap } else { StrokeCap::Butt };
        let end_cap = if index == last || cap == StrokeCap::Butt {
            cap
        } else {
            StrokeCap::Round
        };
        push_capped_segment(instances, p0, p1, r0, r1, start_cap, end_cap, segment_color);
    }
}

fn append_dashed_stroke_instances(
    instances: &mut Vec<MeshInstance>,
    points: &[[f32; 2]],
    width: f32,
    widths: &[f32],
    cap: StrokeCap,
    color: [f32; 4],
    colors: &[[f32; 4]],
    pattern: &[f32],
    dash_offset: f32,
) {
    let cycle: f32 = pattern.iter().copied().sum();
    if cycle <= f32::EPSILON {
        return;
    }
    let per_point = widths.len() == points.len();
    let per_color = colors.len() == points.len();
    let last_point = points.len().saturating_sub(2);
    let mut path_s = 0.0;
    let mut prev_ended_on_at_vertex = false;
    for (index, pair) in points.windows(2).enumerate() {
        let r0 = if per_point {
            widths[index] * 0.5
        } else {
            width * 0.5
        };
        let r1 = if per_point {
            widths[index + 1] * 0.5
        } else {
            width * 0.5
        };
        if !segment_is_drawable(pair[0], pair[1], r0, r1) {
            prev_ended_on_at_vertex = false;
            continue;
        }
        let dx = pair[1][0] - pair[0][0];
        let dy = pair[1][1] - pair[0][1];
        let len = dx.hypot(dy);
        let c0 = if per_color { colors[index] } else { color };
        let c1 = if per_color { colors[index + 1] } else { color };
        let mut local = 0.0;
        while local < len {
            let (on, remaining) = dash_phase(path_s + local + dash_offset, pattern, cycle);
            if remaining <= 1e-5 {
                local += 1e-4;
                continue;
            }
            let take = remaining.min(len - local);
            if on && take > f32::EPSILON {
                let t0 = local / len;
                let t1 = (local + take) / len;
                let q0 = lerp2(pair[0], pair[1], t0);
                let q1 = lerp2(pair[0], pair[1], t1);
                let rr0 = r0 + (r1 - r0) * t0;
                let rr1 = r0 + (r1 - r0) * t1;
                let starts_at_vertex = local <= 1e-5;
                let ends_at_vertex = (local + take) >= len - 1e-5;
                let next_on = if !ends_at_vertex || index == last_point {
                    false
                } else {
                    dash_phase(path_s + len + dash_offset + 1e-4, pattern, cycle).0
                };
                let start_cap = if starts_at_vertex && prev_ended_on_at_vertex {
                    StrokeCap::Butt
                } else {
                    cap
                };
                let end_cap = if ends_at_vertex && next_on {
                    StrokeCap::Butt
                } else {
                    cap
                };
                push_capped_segment(
                    instances,
                    q0,
                    q1,
                    rr0.max(0.0),
                    rr1.max(0.0),
                    start_cap,
                    end_cap,
                    lerp4(c0, c1, t0),
                );
                prev_ended_on_at_vertex = ends_at_vertex;
            } else {
                prev_ended_on_at_vertex = false;
            }
            local += take;
        }
        path_s += len;
    }
}

enum DashPattern {
    Solid,
    Empty,
    Cycle(Vec<f32>),
}

fn normalize_dash(dash: &[f32]) -> DashPattern {
    if dash.is_empty() {
        return DashPattern::Solid;
    }
    if dash.iter().any(|value| !value.is_finite() || *value < 0.0) {
        return DashPattern::Solid;
    }
    if dash.iter().all(|value| *value == 0.0) {
        return DashPattern::Empty;
    }
    let mut pattern = dash.to_vec();
    if pattern.len() % 2 == 1 {
        pattern.extend_from_slice(dash);
    }
    DashPattern::Cycle(pattern)
}

fn dash_phase(distance: f32, pattern: &[f32], cycle: f32) -> (bool, f32) {
    let mut d = distance % cycle;
    if d < 0.0 {
        d += cycle;
    }
    let mut on = true;
    for &len in pattern {
        if d < len {
            return (on, (len - d).max(0.0));
        }
        d -= len;
        on = !on;
    }
    (false, 0.0)
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

fn push_capped_segment(
    instances: &mut Vec<MeshInstance>,
    p0: [f32; 2],
    p1: [f32; 2],
    r0: f32,
    r1: f32,
    start_cap: StrokeCap,
    end_cap: StrokeCap,
    color: [f32; 4],
) {
    let (p0, start_cap) = square_end(p0, p1, r0, start_cap, true);
    let (p1, end_cap) = square_end(p0, p1, r1, end_cap, false);
    push_segment(instances, p0, p1, r0, r1, start_cap, end_cap, color);
}

fn square_end(
    p0: [f32; 2],
    p1: [f32; 2],
    radius: f32,
    cap: StrokeCap,
    start: bool,
) -> ([f32; 2], StrokeCap) {
    if cap != StrokeCap::Square {
        return (if start { p0 } else { p1 }, cap);
    }
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let len = dx.hypot(dy);
    if !len.is_finite() || len < f32::EPSILON {
        return (if start { p0 } else { p1 }, StrokeCap::Butt);
    }
    let tx = dx / len * radius;
    let ty = dy / len * radius;
    if start {
        ([p0[0] - tx, p0[1] - ty], StrokeCap::Butt)
    } else {
        ([p1[0] + tx, p1[1] + ty], StrokeCap::Butt)
    }
}

/// One articulated-line segment. Interior joins keep a single endpoint disc:
/// the previous segment draws it, this segment uses a butt start.
fn push_segment(
    instances: &mut Vec<MeshInstance>,
    p0: [f32; 2],
    p1: [f32; 2],
    r0: f32,
    r1: f32,
    start_cap: StrokeCap,
    end_cap: StrokeCap,
    color: [f32; 4],
) {
    if !segment_is_drawable(p0, p1, r0, r1) {
        return;
    }
    instances.push(MeshInstance::segment(
        color, p0, p1, r0, r1, start_cap, end_cap,
    ));
}

fn segment_is_drawable(p0: [f32; 2], p1: [f32; 2], r0: f32, r1: f32) -> bool {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let length = dx.hypot(dy);
    length.is_finite()
        && length >= f32::EPSILON
        && r0.is_finite()
        && r1.is_finite()
        && (r0 > 0.0 || r1 > 0.0)
}

fn apply_affine_to_instances(instances: &mut [MeshInstance], start: usize, affine: [f32; 6]) {
    if affine == super::clip::IDENTITY_AFFINE {
        return;
    }
    let [a, b, c, d, _, _] = affine;
    let scale = ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) * 0.5;
    for instance in &mut instances[start..] {
        instance.p0 = super::clip::transform_point(affine, instance.p0[0], instance.p0[1]);
        instance.p1 = super::clip::transform_point(affine, instance.p1[0], instance.p1[1]);
        instance.radii[0] *= scale;
        instance.radii[1] *= scale;
    }
}

fn stamp_fragment_clip(instances: &mut [MeshInstance], start: usize, clip: FragmentClip) {
    for instance in &mut instances[start..] {
        instance.clip_rect = clip.rect;
        instance.clip_inv_abcd = clip.inv_abcd;
        instance.clip_inv_ef_cap[0] = clip.inv_ef[0];
        instance.clip_inv_ef_cap[1] = clip.inv_ef[1];
        instance.clip_inv_ef_cap[2] = clip.corner_radius;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extra logical pixels so the covering quad has room for `fwidth` AA.
    const STROKE_AA_FRINGE: f32 = 1.0;

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

    fn sd_variable_capsule(point: [f32; 2], a: [f32; 2], b: [f32; 2], r0: f32, r1: f32) -> f32 {
        capsule_distance(point, a, b) - {
            let pa = [point[0] - a[0], point[1] - a[1]];
            let ba = [b[0] - a[0], b[1] - a[1]];
            let denom = ba[0] * ba[0] + ba[1] * ba[1];
            let t = if denom <= f32::EPSILON {
                0.0
            } else {
                ((pa[0] * ba[0] + pa[1] * ba[1]) / denom).clamp(0.0, 1.0)
            };
            r0 + (r1 - r0) * t
        }
    }

    fn sd_stroke(
        point: [f32; 2],
        a: [f32; 2],
        b: [f32; 2],
        r0: f32,
        r1: f32,
        start: StrokeCap,
        end: StrokeCap,
    ) -> f32 {
        let (a, start) = square_end(a, b, r0, start, true);
        let (b, end) = square_end(a, b, r1, end, false);
        let mut distance = sd_variable_capsule(point, a, b, r0, r1);
        if start == StrokeCap::Butt || end == StrokeCap::Butt {
            let ba = [b[0] - a[0], b[1] - a[1]];
            let seg_len = ba[0].hypot(ba[1]).max(1e-8);
            let local_x =
                (point[0] - a[0]) * (ba[0] / seg_len) + (point[1] - a[1]) * (ba[1] / seg_len);
            if start == StrokeCap::Butt {
                distance = distance.max(-local_x);
            }
            if end == StrokeCap::Butt {
                distance = distance.max(local_x - seg_len);
            }
        }
        distance
    }

    fn covering_corner(
        p0: [f32; 2],
        p1: [f32; 2],
        r0: f32,
        r1: f32,
        start: StrokeCap,
        end: StrokeCap,
        corner: [f32; 2],
    ) -> Option<[f32; 2]> {
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let seg_len = dx.hypot(dy);
        if !seg_len.is_finite() || seg_len < f32::EPSILON {
            return None;
        }
        let tx = dx / seg_len;
        let ty = dy / seg_len;
        let nx = -ty;
        let ny = tx;
        let end0 = match start {
            StrokeCap::Round | StrokeCap::Square => r0 + STROKE_AA_FRINGE,
            StrokeCap::Butt => STROKE_AA_FRINGE,
        };
        let end1 = match end {
            StrokeCap::Round | StrokeCap::Square => r1 + STROKE_AA_FRINGE,
            StrokeCap::Butt => STROKE_AA_FRINGE,
        };
        let side = if corner[0] < 0.0 {
            r0 + STROKE_AA_FRINGE
        } else {
            r1 + STROKE_AA_FRINGE
        };
        let along = if corner[0] < 0.0 {
            -end0
        } else {
            seg_len + end1
        };
        Some([
            p0[0] + tx * along + nx * (corner[1] * side),
            p0[1] + ty * along + ny * (corner[1] * side),
        ])
    }

    #[test]
    fn one_segment_emits_one_instance() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [10.0, 0.0]],
            4.0,
            &[],
            StrokeCap::Round,
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].p0, [0.0, 0.0]);
        assert_eq!(instances[0].p1, [10.0, 0.0]);
        assert_eq!(instances[0].radii, [2.0, 2.0]);
        assert_eq!(
            instances[0].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Round, StrokeCap::Round)
        );
        let pad = 2.0 + STROKE_AA_FRINGE;
        let corner = covering_corner(
            instances[0].p0,
            instances[0].p1,
            2.0,
            2.0,
            StrokeCap::Round,
            StrokeCap::Round,
            [-1.0, -1.0],
        )
        .expect("quad");
        assert!(corner[0] <= -pad + 1e-4);
        assert!(corner[1] <= -pad + 1e-4);
    }

    #[test]
    fn zero_length_and_empty_strokes_emit_nothing() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[3.0, 3.0], [3.0, 3.0]],
            2.0,
            &[],
            StrokeCap::Round,
            [1.0, 1.0, 1.0, 1.0],
        );
        assert!(instances.is_empty());
        append_stroke_instances(
            &mut instances,
            &[[1.0, 1.0]],
            2.0,
            &[],
            StrokeCap::Round,
            [1.0; 4],
        );
        assert!(instances.is_empty());
    }

    #[test]
    fn two_segments_share_an_endpoint_disc() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0]],
            2.0,
            &[],
            StrokeCap::Round,
            [0.0, 0.0, 1.0, 1.0],
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(
            instances[0].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Round, StrokeCap::Round)
        );
        assert_eq!(
            instances[1].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Butt, StrokeCap::Round)
        );
        let join = [8.0, 0.0];
        let first = capsule_distance(join, instances[0].p0, instances[0].p1);
        assert!(first <= instances[0].radii[1] + 1e-5);
        assert!(
            sd_stroke(
                join,
                instances[1].p0,
                instances[1].p1,
                instances[1].radii[0],
                instances[1].radii[1],
                StrokeCap::Butt,
                StrokeCap::Round,
            ) <= 1e-5,
            "join stays on the first segment's end disc, not a second start disc"
        );
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
        let outside_corner = covering_corner(
            p0,
            p1,
            radius,
            radius,
            StrokeCap::Round,
            StrokeCap::Round,
            [-1.0, -1.0],
        )
        .expect("quad");
        assert!(
            capsule_distance(outside_corner, p0, p1) > radius,
            "covering-quad corner must be discarded by the capsule, got {}",
            capsule_distance(outside_corner, p0, p1)
        );
    }

    #[test]
    fn affine_scales_radius_and_moves_endpoints() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [4.0, 0.0]],
            2.0,
            &[],
            StrokeCap::Round,
            [1.0, 1.0, 1.0, 1.0],
        );
        apply_affine_to_instances(&mut instances, 0, [2.0, 0.0, 0.0, 2.0, 5.0, 7.0]);
        assert!((instances[0].p0[0] - 5.0).abs() < 1e-5);
        assert!((instances[0].p0[1] - 7.0).abs() < 1e-5);
        assert!((instances[0].p1[0] - 13.0).abs() < 1e-5);
        assert!((instances[0].radii[0] - 2.0).abs() < 1e-5);
        assert!((instances[0].radii[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn per_point_widths_emit_tapered_radii() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [20.0, 0.0], [20.0, 10.0]],
            2.0,
            &[8.0, 2.0, 4.0],
            StrokeCap::Round,
            [1.0; 4],
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].radii, [4.0, 1.0]);
        assert_eq!(instances[1].radii, [1.0, 2.0]);
    }

    #[test]
    fn mismatched_widths_fall_back_to_uniform_width() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [4.0, 0.0]],
            6.0,
            &[1.0],
            StrokeCap::Round,
            [1.0; 4],
        );
        assert_eq!(instances[0].radii, [3.0, 3.0]);
    }

    #[test]
    fn variable_capsule_matches_constant_radius() {
        let p0 = [0.0, 0.0];
        let p1 = [16.0, 0.0];
        let radius = 2.0;
        for point in [[8.0, 0.0], [0.0, 0.0], [16.0, 1.0], [8.0, 3.0], [-1.0, 0.0]] {
            let constant = capsule_distance(point, p0, p1) - radius;
            let variable = sd_variable_capsule(point, p0, p1, radius, radius);
            assert!(
                (constant - variable).abs() < 2e-4,
                "point {point:?}: constant {constant} vs variable {variable}"
            );
        }
    }

    #[test]
    fn variable_capsule_keeps_large_end_and_thins() {
        let p0 = [0.0, 0.0];
        let p1 = [20.0, 0.0];
        assert!(sd_variable_capsule([0.0, 3.0], p0, p1, 4.0, 1.0) < 0.0);
        assert!(sd_variable_capsule([20.0, 0.4], p0, p1, 4.0, 1.0) < 0.0);
        assert!(sd_variable_capsule([20.0, 2.0], p0, p1, 4.0, 1.0) > 0.0);
        assert!(sd_variable_capsule([10.0, 0.0], p0, p1, 4.0, 1.0) < 0.0);
    }

    #[test]
    fn butt_cap_cuts_the_endpoint_disc() {
        let p0 = [0.0, 0.0];
        let p1 = [20.0, 0.0];
        let radius = 2.0;
        assert!(
            sd_stroke(
                [10.0, 0.0],
                p0,
                p1,
                radius,
                radius,
                StrokeCap::Butt,
                StrokeCap::Butt
            ) < 0.0
        );
        assert!(
            sd_stroke(
                [0.0, 0.0],
                p0,
                p1,
                radius,
                radius,
                StrokeCap::Butt,
                StrokeCap::Butt
            ) <= 1e-5
        );
        assert!(
            sd_stroke(
                [-0.5, 0.0],
                p0,
                p1,
                radius,
                radius,
                StrokeCap::Butt,
                StrokeCap::Butt
            ) > 0.0,
            "butt must reject the round-cap disc past the start"
        );
        assert!(capsule_distance([-0.5, 0.0], p0, p1) < radius);
        let butt_corner = covering_corner(
            p0,
            p1,
            radius,
            radius,
            StrokeCap::Butt,
            StrokeCap::Butt,
            [-1.0, 0.0],
        )
        .expect("quad");
        assert!(
            butt_corner[0] > -radius,
            "butt covering quad must not extend a full radius past the start, got {}",
            butt_corner[0]
        );
    }

    #[test]
    fn tapered_covering_quad_is_a_trapezoid() {
        let thick = covering_corner(
            [0.0, 0.0],
            [10.0, 0.0],
            4.0,
            1.0,
            StrokeCap::Round,
            StrokeCap::Round,
            [-1.0, 1.0],
        )
        .expect("quad");
        let thin = covering_corner(
            [0.0, 0.0],
            [10.0, 0.0],
            4.0,
            1.0,
            StrokeCap::Round,
            StrokeCap::Round,
            [1.0, 1.0],
        )
        .expect("quad");
        assert!(
            thick[1] >= 4.0 + STROKE_AA_FRINGE - 1e-4,
            "start side pad follows r0, got {}",
            thick[1]
        );
        assert!(
            (thin[1] - (1.0 + STROKE_AA_FRINGE)).abs() < 1e-4,
            "end side pad follows r1, got {}",
            thin[1]
        );
    }

    #[test]
    fn interior_round_join_omits_the_second_start_disc() {
        let mut instances = Vec::new();
        append_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            2.0,
            &[],
            StrokeCap::Round,
            [1.0; 4],
        );
        assert_eq!(instances.len(), 3);
        assert_eq!(
            instances[0].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Round, StrokeCap::Round)
        );
        assert_eq!(
            instances[1].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Butt, StrokeCap::Round)
        );
        assert_eq!(
            instances[2].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Butt, StrokeCap::Round)
        );
    }

    fn solid_style<'a>(width: f32, widths: &'a [f32], cap: StrokeCap) -> StrokeStyle<'a> {
        StrokeStyle {
            width,
            widths,
            cap,
            dash: &[],
            dash_offset: 0.0,
            colors: &[],
        }
    }

    #[test]
    fn unused_attributes_match_legacy_solid_emit() {
        let points = [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0]];
        let color = [1.0, 0.0, 0.0, 1.0];
        let mut legacy = Vec::new();
        append_stroke_instances(&mut legacy, &points, 2.0, &[], StrokeCap::Round, color);
        let mut via_style = Vec::new();
        emit_stroke_instances(
            &mut via_style,
            &points,
            solid_style(2.0, &[], StrokeCap::Round),
            color,
        );
        assert_eq!(
            via_style, legacy,
            "empty dash/colors and Round must reuse the solid instance loop"
        );
        emit_stroke_instances(
            &mut via_style,
            &points,
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Round,
                dash: &[-4.0, 2.0],
                dash_offset: 3.0,
                colors: &[[0.0, 1.0, 0.0, 1.0]],
            },
            color,
        );
        assert_eq!(
            via_style[legacy.len()..],
            legacy,
            "invalid dash and mismatched colors must stay on the solid emit"
        );
    }

    #[test]
    fn square_cap_expands_endpoints_onto_the_butt_gpu_path() {
        let mut instances = Vec::new();
        emit_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [10.0, 0.0]],
            solid_style(4.0, &[], StrokeCap::Square),
            [1.0; 4],
        );
        assert_eq!(instances.len(), 1);
        assert!((instances[0].p0[0] + 2.0).abs() < 1e-5);
        assert!((instances[0].p1[0] - 12.0).abs() < 1e-5);
        assert_eq!(
            instances[0].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Butt, StrokeCap::Butt)
        );
        assert!(
            sd_stroke(
                [-1.5, 1.5],
                [0.0, 0.0],
                [10.0, 0.0],
                2.0,
                2.0,
                StrokeCap::Square,
                StrokeCap::Square,
            ) < 0.0
        );
        assert!(
            sd_stroke(
                [-1.5, 1.5],
                [0.0, 0.0],
                [10.0, 0.0],
                2.0,
                2.0,
                StrokeCap::Round,
                StrokeCap::Round,
            ) > 0.0
        );
        assert!(
            sd_stroke(
                [-2.5, 0.0],
                [0.0, 0.0],
                [10.0, 0.0],
                2.0,
                2.0,
                StrokeCap::Square,
                StrokeCap::Square,
            ) > 0.0
        );
    }

    #[test]
    fn dash_emits_only_on_segments() {
        let mut instances = Vec::new();
        emit_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [40.0, 0.0]],
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Butt,
                dash: &[10.0, 10.0],
                dash_offset: 0.0,
                colors: &[],
            },
            [1.0; 4],
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].p0, [0.0, 0.0]);
        assert!((instances[0].p1[0] - 10.0).abs() < 1e-4);
        assert!((instances[1].p0[0] - 20.0).abs() < 1e-4);
        assert!((instances[1].p1[0] - 30.0).abs() < 1e-4);
    }

    #[test]
    fn odd_dash_repeats_and_zero_pattern_emits_nothing() {
        let mut odd = Vec::new();
        emit_stroke_instances(
            &mut odd,
            &[[0.0, 0.0], [40.0, 0.0]],
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Butt,
                dash: &[10.0],
                dash_offset: 0.0,
                colors: &[],
            },
            [1.0; 4],
        );
        assert_eq!(odd.len(), 2);
        let mut empty = Vec::new();
        emit_stroke_instances(
            &mut empty,
            &[[0.0, 0.0], [40.0, 0.0]],
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Round,
                dash: &[0.0, 0.0],
                dash_offset: 0.0,
                colors: &[],
            },
            [1.0; 4],
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn continuous_dash_keeps_join_once_across_vertices() {
        let mut instances = Vec::new();
        emit_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0]],
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Round,
                dash: &[100.0, 4.0],
                dash_offset: 0.0,
                colors: &[],
            },
            [1.0; 4],
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(
            instances[0].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Round, StrokeCap::Butt)
        );
        assert_eq!(
            instances[1].clip_inv_ef_cap[3],
            pack_caps(StrokeCap::Butt, StrokeCap::Round)
        );
    }

    #[test]
    fn per_point_colors_use_the_segment_start() {
        let mut instances = Vec::new();
        emit_stroke_instances(
            &mut instances,
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 8.0]],
            StrokeStyle {
                width: 2.0,
                widths: &[],
                cap: StrokeCap::Round,
                dash: &[],
                dash_offset: 0.0,
                colors: &[
                    [1.0, 0.0, 0.0, 1.0],
                    [0.0, 1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0, 1.0],
                ],
            },
            [0.0, 0.0, 1.0, 1.0],
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(instances[1].color, [0.0, 1.0, 0.0, 1.0]);
    }
}
