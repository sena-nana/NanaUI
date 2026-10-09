//! Chart marks (`nana-ui-charts`): the arrays a chart layout built, kept on
//! the GPU per chart while their revision holds, and drawn by `chart.wgsl`.
//!
//! A chart costs an upload when its layout changes and a 256-byte uniform
//! write when its placement or hover does. Motion and emphasis are sampled
//! from the motion clock in the shaders, so a frame in which only time moves
//! writes nothing here.

use std::collections::{HashMap, HashSet};

use bytemuck::{Pod, Zeroable};
use nana_ui_charts::{ChartMarks, GpuStyle, MarkPass, draw_flags};
use nana_ui_core::Easing;
use nana_ui_scene::{PrimitiveId, SceneChartHover};

use super::clip::FragmentClip;
use super::color::{orthographic_scaled, pack_linear};
use super::mesh::GpuClip;
use crate::PhysicalRect;

const SAMPLE_COUNTS: [u32; 2] = [1, 4];

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct ChartGlobals {
    transform: [f32; 16],
    viewport_scale: f32,
    corner_exponent: f32,
    _pad: [f32; 2],
}

/// Matches `ChartUniform` in `chart.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct ChartUniform {
    affine_abcd: [f32; 4],
    affine_ef_origin: [f32; 4],
    plot: [f32; 4],
    clip: GpuClip,
    timing: [f32; 4],
    timing_u: [u32; 4],
    bezier: [f32; 4],
    hover: [u32; 4],
    focus: [u32; 4],
    params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ChartUniform>() == 256);

/// Matches `GpuDraw` in `chart.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuDraw {
    a: [u32; 4],
    b: [u32; 4],
}

fn pass_code(pass: MarkPass) -> u32 {
    match pass {
        MarkPass::Line => 0,
        MarkPass::Area => 1,
        MarkPass::Shapes => 2,
    }
}

fn easing_code(easing: Easing) -> (u32, [f32; 4]) {
    match easing {
        Easing::Linear => (0, [0.0; 4]),
        Easing::EaseOutCubic => (1, [0.0; 4]),
        Easing::EaseInOutCubic => (2, [0.0; 4]),
        Easing::CubicBezier(points) => (3, points),
    }
}

fn split(time: std::time::Duration) -> (u32, f32) {
    nana_ui_core::motion::glyph::split_seconds(time)
}

/// The three chart pipelines at both sample counts, built on the first
/// chart a painter meets.
pub(super) struct ChartPipeline {
    line: [wgpu::RenderPipeline; 2],
    area: [wgpu::RenderPipeline; 2],
    shape: [wgpu::RenderPipeline; 2],
    layout: wgpu::BindGroupLayout,
}

fn storage(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32, size: usize) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size as u64),
        },
        count: None,
    }
}

impl ChartPipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        motion_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.chart.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("shader/chart.wgsl"),
                "\n",
                include_str!("shader/color.wgsl"),
            ))),
        });
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.chart.layout"),
            entries: &[
                uniform(0, std::mem::size_of::<ChartGlobals>()),
                storage(1, both),
                storage(2, wgpu::ShaderStages::VERTEX),
                storage(3, both),
                uniform(4, std::mem::size_of::<ChartUniform>()),
                storage(5, both),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.chart.pipeline"),
            bind_group_layouts: &[Some(&layout), Some(motion_layout)],
            immediate_size: 0,
        });
        let build = |vs: &str, fs: &str, samples: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("nana-ui.scene.chart"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    // Every edge is analytic coverage; MSAA samples
                    // geometry only, so both sample counts share it.
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let family = |vs: &str, fs: &str| SAMPLE_COUNTS.map(|samples| build(vs, fs, samples));
        Self {
            line: family("chart_line_vs", "chart_line_fs"),
            area: family("chart_area_vs", "chart_area_fs"),
            shape: family("chart_shape_vs", "chart_shape_fs"),
            layout,
        }
    }
}

/// One chart's arrays and uniform on the GPU.
struct ChartGpu {
    revision: u64,
    /// Points, shapes, styles and draws, kept while a new revision fits.
    arrays: [wgpu::Buffer; 4],
    uniform: wgpu::Buffer,
    uploaded_uniform: Option<ChartUniform>,
    bind_group: wgpu::BindGroup,
    /// `(pass, first, count, closed)` per draw.
    passes: Vec<(MarkPass, u32, u32, bool)>,
}

/// A painter target's charts.
#[derive(Default)]
pub(super) struct ChartPipelineTarget {
    globals: Option<wgpu::Buffer>,
    uploaded_globals: Option<ChartGlobals>,
    charts: HashMap<PrimitiveId, ChartGpu>,
    /// The charts the prepared batch's commands name, by slot.
    slots: Vec<PrimitiveId>,
    seen: HashSet<PrimitiveId>,
}

/// Where and how one chart is drawn this frame.
pub(super) struct ChartPlacement<'a> {
    pub marks: &'a ChartMarks,
    pub origin: [f32; 2],
    pub hover: &'a SceneChartHover,
    pub affine: [f32; 6],
    pub opacity: f32,
    pub clip: FragmentClip,
    pub viewport_scale: f32,
}

fn storage_buffer(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
    // A binding may not be empty: an empty array still gets room for one.
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(64).next_multiple_of(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn write(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    bytes: &[u8],
    work: Option<&crate::gpu_work::GpuWorkSink>,
) {
    if bytes.is_empty() {
        return;
    }
    match work {
        Some(work) => work.write_buffer(queue, buffer, 0, bytes),
        None => queue.write_buffer(buffer, 0, bytes),
    }
}

impl ChartPipelineTarget {
    pub(super) fn begin_frame(&mut self) {
        self.slots.clear();
        self.seen.clear();
    }

    /// Drop the charts the frame no longer draws.
    pub(super) fn finish_frame(&mut self) {
        let seen = &self.seen;
        self.charts.retain(|id, _| seen.contains(id));
    }

    /// The globals every chart shares: the target's projection.
    pub(super) fn upload_globals(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        physical_size: [u32; 2],
        scale: f32,
        corner_exponent: f32,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let globals = ChartGlobals {
            transform: orthographic_scaled(physical_size[0], physical_size[1], scale),
            viewport_scale: scale,
            corner_exponent,
            _pad: [0.0; 2],
        };
        let buffer = self.globals.get_or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.chart.globals"),
                size: std::mem::size_of::<ChartGlobals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        if self.uploaded_globals != Some(globals) {
            write(queue, buffer, bytemuck::bytes_of(&globals), work);
            self.uploaded_globals = Some(globals);
        }
    }

    /// Bring `id`'s arrays and uniform up to date, and name it for this
    /// frame's draws. `None` when the chart draws nothing.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        pipeline: &ChartPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: PrimitiveId,
        placement: ChartPlacement<'_>,
        corner_exponent: f32,
        physical_size: [u32; 2],
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) -> Option<u32> {
        let marks = placement.marks;
        if marks.draws.is_empty() || placement.opacity <= 0.0 {
            return None;
        }
        self.upload_globals(
            device,
            queue,
            physical_size,
            placement.viewport_scale,
            corner_exponent,
            work,
        );
        let globals = self.globals.as_ref().expect("chart globals");
        let stale = self
            .charts
            .get(&id)
            .is_none_or(|gpu| gpu.revision != marks.revision);
        if stale {
            let previous = self.charts.remove(&id);
            self.charts.insert(
                id,
                upload_marks(pipeline, device, queue, globals, marks, previous, work),
            );
            if let Some(work) = work {
                work.record_batch_rebuild();
            }
        }
        let gpu = self.charts.get_mut(&id).expect("chart uploaded");
        let uniform = uniform_of(&placement);
        if gpu.uploaded_uniform != Some(uniform) {
            write(queue, &gpu.uniform, bytemuck::bytes_of(&uniform), work);
            gpu.uploaded_uniform = Some(uniform);
        }
        self.seen.insert(id);
        let slot = self.slots.len() as u32;
        self.slots.push(id);
        Some(slot)
    }

    /// How many draws the chart in `slot` has.
    pub(super) fn draw_count(&self, slot: u32) -> u32 {
        self.slots
            .get(slot as usize)
            .and_then(|id| self.charts.get(id))
            .map_or(0, |gpu| gpu.passes.len() as u32)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        pipeline: &ChartPipeline,
        pass: &mut wgpu::RenderPass<'_>,
        slot: u32,
        draw: u32,
        scissor: PhysicalRect,
        sample_count: u32,
        motion: &wgpu::BindGroup,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(gpu) = self
            .slots
            .get(slot as usize)
            .and_then(|id| self.charts.get(id))
        else {
            return;
        };
        let Some(&(kind, first, count, closed)) = gpu.passes.get(draw as usize) else {
            return;
        };
        let msaa = usize::from(sample_count > 1);
        let (pipeline, instances) = match kind {
            MarkPass::Line => {
                let segments = if closed {
                    count
                } else {
                    count.saturating_sub(1)
                };
                (&pipeline.line[msaa], 0..segments)
            }
            MarkPass::Area => {
                let segments = if closed {
                    count
                } else {
                    count.saturating_sub(1)
                };
                (&pipeline.area[msaa], 0..segments)
            }
            MarkPass::Shapes => (&pipeline.shape[msaa], first..first + count),
        };
        if instances.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &gpu.bind_group, &[]);
        pass.set_bind_group(1, motion, &[]);
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.draw(draw * 6..draw * 6 + 6, instances);
        if let Some(work) = work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}

/// Writes a new revision's arrays, into the previous revision's buffers
/// where they fit (a streaming chart grows by a point at a time).
fn upload_marks(
    pipeline: &ChartPipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    globals: &wgpu::Buffer,
    marks: &ChartMarks,
    previous: Option<ChartGpu>,
    work: Option<&crate::gpu_work::GpuWorkSink>,
) -> ChartGpu {
    let styles: Vec<GpuStyle> = marks
        .styles
        .iter()
        .map(|style| GpuStyle {
            fill: pack_linear(style.fill),
            fill_end: pack_linear(style.fill_end),
            stroke: pack_linear(style.stroke),
            params: style.params,
        })
        .collect();
    let draws: Vec<GpuDraw> = marks
        .draws
        .iter()
        .map(|draw| GpuDraw {
            a: [draw.first, draw.count, draw.style, draw.flags],
            b: [draw.neighbours.max(1), draw.series, pass_code(draw.pass), 0],
        })
        .collect();
    let bytes: [&[u8]; 4] = [
        bytemuck::cast_slice(&marks.points),
        bytemuck::cast_slice(&marks.shapes),
        bytemuck::cast_slice(&styles),
        bytemuck::cast_slice(&draws),
    ];
    const LABELS: [&str; 4] = [
        "nana-ui.scene.chart.points",
        "nana-ui.scene.chart.shapes",
        "nana-ui.scene.chart.styles",
        "nana-ui.scene.chart.draws",
    ];
    let (old_arrays, uniform, uploaded_uniform, old_bind_group) = match previous {
        Some(gpu) => (
            Some(gpu.arrays),
            gpu.uniform,
            gpu.uploaded_uniform,
            Some(gpu.bind_group),
        ),
        None => (
            None,
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.chart.uniform"),
                size: std::mem::size_of::<ChartUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            None,
            None,
        ),
    };
    // A new revision writes into the old buffers where it fits; a stream
    // grows into room left for it.
    let mut grew = old_arrays.is_none();
    let mut old_arrays = old_arrays.map(|arrays| arrays.map(Some));
    let arrays: [wgpu::Buffer; 4] = std::array::from_fn(|index| {
        let needed = bytes[index].len() as u64;
        match old_arrays.as_mut().and_then(|arrays| arrays[index].take()) {
            Some(buffer) if needed <= buffer.size() => buffer,
            kept => {
                grew = true;
                let room = if kept.is_some() {
                    needed.next_power_of_two()
                } else {
                    needed
                };
                storage_buffer(device, LABELS[index], room)
            }
        }
    });
    for (buffer, bytes) in arrays.iter().zip(bytes) {
        write(queue, buffer, bytes, work);
    }
    let bind_group = match old_bind_group {
        Some(bind_group) if !grew => bind_group,
        _ => {
            if let Some(work) = work {
                work.record_realloc();
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui.scene.chart.bind_group"),
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: arrays[0].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: arrays[1].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: arrays[2].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: arrays[3].as_entire_binding(),
                    },
                ],
            })
        }
    };
    ChartGpu {
        revision: marks.revision,
        arrays,
        uniform,
        uploaded_uniform,
        bind_group,
        passes: marks
            .draws
            .iter()
            .map(|draw| {
                (
                    draw.pass,
                    draw.first,
                    draw.count,
                    draw.flags & draw_flags::CLOSED != 0,
                )
            })
            .collect(),
    }
}

fn uniform_of(placement: &ChartPlacement<'_>) -> ChartUniform {
    let [a, b, c, d, e, f] = placement.affine;
    let sigma = (a * d - b * c).abs().sqrt().max(1e-4);
    let marks = placement.marks;
    let (start_secs, start_sub, duration, easing, reveal, animating) = match marks.transition {
        Some(motion) => {
            let (secs, sub) = split(motion.start);
            (
                secs,
                sub,
                motion.duration,
                motion.easing,
                motion.reveal,
                true,
            )
        }
        None => (0, 0.0, 0.0, Easing::Linear, false, false),
    };
    let (easing, bezier) = easing_code(easing);
    let (hover_secs, hover_sub) = split(placement.hover.since);
    let hover = placement.hover;
    ChartUniform {
        affine_abcd: [a, b, c, d],
        affine_ef_origin: [e, f, placement.origin[0], placement.origin[1]],
        plot: marks.plot,
        clip: GpuClip::from_fragment(placement.clip),
        timing: [start_sub, duration, hover_sub, hover.duration],
        timing_u: [
            start_secs,
            easing,
            u32::from(reveal) | (u32::from(animating) << 1),
            hover_secs,
        ],
        bezier,
        hover: [
            hover.current[0],
            hover.current[1],
            hover.previous[0],
            hover.previous[1],
        ],
        focus: [hover.focus[0], hover.focus[1], 0, 0],
        params: [
            placement.opacity.clamp(0.0, 1.0),
            placement.viewport_scale * sigma,
            hover.growth,
            0.0,
        ],
    }
}
