use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use iced::widget::shader;
use iced::{Color, Rectangle, wgpu};

use crate::geometry::{LogicalRect, PhysicalRect};

const SHADER_SOURCE: &str = r#"
struct ViewUniform {
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    parameters: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> view: ViewUniform;

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
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    output.uv = positions[index] * 0.5 + vec2<f32>(0.5);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let seed = view.parameters.x;
    let wave = 0.5 + 0.5 * sin((uv.x * 5.5 + uv.y * 3.0 + seed) * 3.14159265);
    let radial = smoothstep(0.82, 0.05, distance(uv, vec2<f32>(0.64, 0.42)));
    let grid_x = 1.0 - smoothstep(0.0, 0.035, abs(fract(uv.x * 12.0) - 0.5));
    let grid_y = 1.0 - smoothstep(0.0, 0.035, abs(fract(uv.y * 8.0) - 0.5));
    let grid = max(grid_x, grid_y) * 0.08;
    let mix_amount = clamp(0.18 + wave * 0.34 + radial * 0.28, 0.0, 1.0);
    let color = mix(view.color_a.rgb, view.color_b.rgb, mix_amount) + grid;
    return vec4<f32>(color, 1.0);
}
"#;

/// A stable logical/physical region suitable for a viewport and scissor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSlot {
    pub id: u64,
    pub logical: LogicalRect,
    pub physical: PhysicalRect,
}

impl RenderSlot {
    pub fn new(id: u64, logical: LogicalRect, scale_factor: f32) -> Self {
        let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let left = (logical.x * scale_factor).floor().max(0.0);
        let top = (logical.y * scale_factor).floor().max(0.0);
        let right = ((logical.x + logical.width) * scale_factor)
            .ceil()
            .max(left);
        let bottom = ((logical.y + logical.height) * scale_factor)
            .ceil()
            .max(top);

        Self {
            id,
            logical,
            physical: PhysicalRect {
                x: saturating_u32(left),
                y: saturating_u32(top),
                width: saturating_u32(right - left),
                height: saturating_u32(bottom - top),
            },
        }
    }

    pub fn clipped_physical(self, target_width: u32, target_height: u32) -> PhysicalRect {
        let right = self
            .physical
            .x
            .saturating_add(self.physical.width)
            .min(target_width);
        let bottom = self
            .physical
            .y
            .saturating_add(self.physical.height)
            .min(target_height);
        let x = self.physical.x.min(target_width);
        let y = self.physical.y.min(target_height);

        PhysicalRect {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuViewPalette {
    pub background: Color,
    pub accent: Color,
}

/// Selects how a [`GpuView`] contributes commands to the current frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GpuViewMode {
    /// Reuse Iced's current render pass for simple content.
    #[default]
    Inline,
    /// Open a dedicated render pass with Iced's command encoder.
    Standalone,
}

/// A reusable custom primitive rendered by Iced's WGPU renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuView {
    id: u64,
    palette: GpuViewPalette,
    revision: u32,
    mode: GpuViewMode,
}

impl GpuView {
    pub const fn new(id: u64, palette: GpuViewPalette, revision: u32) -> Self {
        Self {
            id,
            palette,
            revision,
            mode: GpuViewMode::Inline,
        }
    }

    pub const fn mode(mut self, mode: GpuViewMode) -> Self {
        self.mode = mode;
        self
    }
}

impl<Message> shader::Program<Message> for GpuView {
    type State = ();
    type Primitive = GpuViewPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        GpuViewPrimitive {
            id: self.id,
            mode: self.mode,
            uniform: ViewUniform {
                color_a: color_array(self.palette.background),
                color_b: color_array(self.palette.accent),
                parameters: [self.revision as f32 * 0.17, 0.0, 0.0, 0.0],
            },
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
    color_a: [f32; 4],
    color_b: [f32; 4],
    parameters: [f32; 4],
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct GpuViewPrimitive {
    id: u64,
    mode: GpuViewMode,
    uniform: ViewUniform,
}

impl shader::Primitive for GpuViewPrimitive {
    type Pipeline = GpuViewPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let (slot, viewport_rect) = slot_for_bounds(self.id, bounds, viewport.scale_factor());
        let entry = pipeline.slots.entry(self.id).or_insert_with(|| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui gpu view uniform"),
                size: std::mem::size_of::<ViewUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui gpu view bind group"),
                layout: &pipeline.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            PreparedSlot {
                buffer,
                bind_group,
                slot,
                viewport: viewport_rect,
                used: true,
            }
        });
        entry.slot = slot;
        entry.viewport = viewport_rect;
        entry.used = true;
        queue.write_buffer(&entry.buffer, 0, bytemuck::bytes_of(&self.uniform));
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        if self.mode == GpuViewMode::Standalone {
            return false;
        }

        let Some(slot) = pipeline.slots.get(&self.id) else {
            return false;
        };
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &slot.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        if self.mode != GpuViewMode::Standalone {
            return;
        }

        let Some(slot) = pipeline.slots.get(&self.id) else {
            return;
        };
        let bounds = intersect_physical(slot.slot.physical, *clip_bounds);
        if bounds.width == 0 || bounds.height == 0 {
            return;
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui gpu view standalone render pass"),
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
        let viewport = slot.viewport;
        render_pass.set_viewport(viewport[0], viewport[1], viewport[2], viewport[3], 0.0, 1.0);
        render_pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &slot.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[doc(hidden)]
pub struct GpuViewPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    slots: HashMap<u64, PreparedSlot>,
}

impl shader::Pipeline for GpuViewPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui gpu view shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui gpu view bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui gpu view pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui gpu view pipeline"),
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
                    blend: Some(wgpu::BlendState::REPLACE),
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

        Self {
            pipeline,
            bind_group_layout,
            slots: HashMap::new(),
        }
    }

    fn trim(&mut self) {
        self.slots.retain(|_, slot| {
            let retain = slot.used;
            slot.used = false;
            retain
        });
    }
}

struct PreparedSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    slot: RenderSlot,
    viewport: [f32; 4],
    used: bool,
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn saturating_u32(value: f32) -> u32 {
    value.clamp(0.0, u32::MAX as f32) as u32
}

pub(crate) fn slot_for_bounds(
    id: u64,
    bounds: &Rectangle,
    scale_factor: f32,
) -> (RenderSlot, [f32; 4]) {
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let logical = LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height);
    let slot = RenderSlot::new(id, logical, scale_factor);

    (
        slot,
        [
            logical.x * scale_factor,
            logical.y * scale_factor,
            logical.width * scale_factor,
            logical.height * scale_factor,
        ],
    )
}

fn intersect_physical(slot: PhysicalRect, clip: Rectangle<u32>) -> PhysicalRect {
    let left = slot.x.max(clip.x);
    let top = slot.y.max(clip.y);
    let right = slot
        .x
        .saturating_add(slot.width)
        .min(clip.x.saturating_add(clip.width));
    let bottom = slot
        .y
        .saturating_add(slot.height)
        .min(clip.y.saturating_add(clip.height));

    PhysicalRect {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GpuView, GpuViewMode, GpuViewPalette, RenderSlot, intersect_physical, slot_for_bounds,
    };
    use crate::geometry::{LogicalRect, PhysicalRect};
    use iced::{Color, Rectangle};

    #[test]
    fn render_slot_covers_fractional_physical_edges_and_clips_to_target() {
        let slot = RenderSlot::new(7, LogicalRect::new(10.25, 20.5, 100.5, 50.25), 1.5);
        assert_eq!(slot.physical.x, 15);
        assert_eq!(slot.physical.y, 30);
        assert_eq!(slot.physical.width, 152);
        assert_eq!(slot.physical.height, 77);

        let clipped = slot.clipped_physical(120, 90);
        assert_eq!(clipped.x, 15);
        assert_eq!(clipped.y, 30);
        assert_eq!(clipped.width, 105);
        assert_eq!(clipped.height, 60);
    }

    #[test]
    fn render_slot_sanitizes_scale_and_never_underflows_when_outside_target() {
        let slot = RenderSlot::new(1, LogicalRect::new(50.0, 60.0, 20.0, 30.0), f32::NAN);
        assert_eq!(slot.physical.width, 20);
        assert_eq!(slot.physical.height, 30);
        assert_eq!(
            slot.clipped_physical(10, 10),
            PhysicalRect {
                x: 10,
                y: 10,
                width: 0,
                height: 0,
            }
        );
    }

    #[test]
    fn standalone_mode_is_explicit_and_preserves_the_instance_identity() {
        let view = GpuView::new(
            42,
            GpuViewPalette {
                background: Color::BLACK,
                accent: Color::WHITE,
            },
            3,
        )
        .mode(GpuViewMode::Standalone);

        assert_eq!(view.id, 42);
        assert_eq!(view.revision, 3);
        assert_eq!(view.mode, GpuViewMode::Standalone);
    }

    #[test]
    fn physical_intersection_rejects_clipped_and_overlapping_edges() {
        let slot = PhysicalRect {
            x: 20,
            y: 30,
            width: 100,
            height: 80,
        };
        assert_eq!(
            intersect_physical(
                slot,
                Rectangle {
                    x: 50,
                    y: 10,
                    width: 100,
                    height: 60,
                },
            ),
            PhysicalRect {
                x: 50,
                y: 30,
                width: 70,
                height: 40,
            }
        );
        assert_eq!(
            intersect_physical(
                slot,
                Rectangle {
                    x: 200,
                    y: 200,
                    width: 10,
                    height: 10,
                },
            ),
            PhysicalRect {
                x: 200,
                y: 200,
                width: 0,
                height: 0,
            }
        );
    }

    #[test]
    fn transformed_bounds_keep_size_when_moved_and_clipped() {
        let original = Rectangle {
            x: 20.0,
            y: 30.0,
            width: 100.0,
            height: 80.0,
        };
        let (_, original_viewport) = slot_for_bounds(7, &original, 1.5);
        let mut moved = original;
        moved.x = 120.0;
        let (_, moved_viewport) = slot_for_bounds(7, &moved, 1.5);
        moved.x = -40.0;
        let (clipped_slot, clipped_viewport) = slot_for_bounds(7, &moved, 1.5);

        assert_eq!(original_viewport, [30.0, 45.0, 150.0, 120.0]);
        assert_eq!(moved_viewport, [180.0, 45.0, 150.0, 120.0]);
        assert_eq!(clipped_viewport, [-60.0, 45.0, 150.0, 120.0]);
        assert_eq!(
            intersect_physical(
                clipped_slot.physical,
                Rectangle {
                    x: 0,
                    y: 0,
                    width: 90,
                    height: 240,
                },
            )
            .width,
            90
        );
    }
}
