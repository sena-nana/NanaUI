//! Snapshot-only host texture and `"gpu-view"` Scene painter.
//!
//! Uses the snapshot Device/Queue. Not product code.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use iced::Color;
use iced_wgpu::wgpu;
use nana_ui::runtime::GPU_VIEW_RENDERER;
use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureRegistry, SceneGpuNode, SceneGpuPrepareContext,
    SceneGpuRenderContext, SceneGpuRenderer, SceneGpuRendererRegistry,
};
use nana_ui_scene::PrimitiveId;

pub const SNAPSHOT_GPU_SLOT: &str = "snapshot-gpu";
const TEXTURE_SIZE: u32 = 32;
const GPU_VIEW_SHADER: &str = r#"
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

pub struct SnapshotGpu {
    _texture: wgpu::Texture,
    pub host_texture: HostTexture,
    pub textures: HostTextureRegistry,
    pub renderers: SceneGpuRendererRegistry,
}

pub fn create_snapshot_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    background: Color,
    accent: Color,
) -> SnapshotGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui snapshot host texture"),
        size: wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let bytes_per_row = TEXTURE_SIZE * 4;
    let aligned_row = bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let pixel = color_rgba8(accent);
    let mut pixels = vec![0_u8; aligned_row as usize * TEXTURE_SIZE as usize];
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let offset = (y * aligned_row + x * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_row),
            rows_per_image: Some(TEXTURE_SIZE),
        },
        wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
    );
    let host_texture = HostTexture::from_wgpu(1, 0, texture.create_view(&Default::default()));
    let textures = HostTextureRegistry::new();
    textures.register(
        SNAPSHOT_GPU_SLOT,
        host_texture.clone(),
        TEXTURE_SIZE,
        TEXTURE_SIZE,
        HostTextureAlphaMode::Opaque,
    );
    let mut renderers = SceneGpuRendererRegistry::new();
    renderers.insert(
        GPU_VIEW_RENDERER,
        Arc::new(SnapshotGpuViewRenderer {
            background: color_array(background),
            accent: color_array(accent),
            state: Mutex::new(None),
        }),
    );
    SnapshotGpu {
        _texture: texture,
        host_texture,
        textures,
        renderers,
    }
}

struct SnapshotGpuViewRenderer {
    background: [f32; 4],
    accent: [f32; 4],
    state: Mutex<Option<PreparedGpuView>>,
}

impl fmt::Debug for SnapshotGpuViewRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotGpuViewRenderer")
            .field("background", &self.background)
            .field("accent", &self.accent)
            .finish_non_exhaustive()
    }
}

impl SceneGpuRenderer for SnapshotGpuViewRenderer {
    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>) {
        let mut state = self.state.lock().expect("snapshot gpu-view pipeline");
        let prepared = state
            .get_or_insert_with(|| PreparedGpuView::new(context.device, context.target_format));
        if prepared.format != context.target_format {
            *prepared = PreparedGpuView::new(context.device, context.target_format);
        }
        let scale = if context.scale_factor.is_finite() && context.scale_factor > 0.0 {
            context.scale_factor
        } else {
            1.0
        };
        let viewport = [
            context.bounds.x * scale,
            context.bounds.y * scale,
            context.bounds.width * scale,
            context.bounds.height * scale,
        ];
        let uniform = ViewUniform {
            color_a: self.background,
            color_b: self.accent,
            parameters: [node.custom.revision as f32 * 0.17, 0.0, 0.0, 0.0],
        };
        if !prepared.slots.contains_key(&node.id) {
            let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui snapshot gpu-view uniform"),
                size: std::mem::size_of::<ViewUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("nana-ui snapshot gpu-view bind group"),
                    layout: &prepared.bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
            prepared.slots.insert(
                node.id,
                PreparedSlot {
                    buffer,
                    bind_group,
                    viewport,
                },
            );
        }
        let entry = prepared
            .slots
            .get_mut(&node.id)
            .expect("snapshot gpu-view slot prepared");
        entry.viewport = viewport;
        context
            .queue
            .write_buffer(&entry.buffer, 0, &uniform_bytes(&uniform));
    }

    fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>) {
        let state = self.state.lock().expect("snapshot gpu-view pipeline");
        let Some(prepared) = state.as_ref() else {
            return;
        };
        let Some(slot) = prepared.slots.get(&node.id) else {
            return;
        };
        if context.bounds.width == 0 || context.bounds.height == 0 {
            return;
        }
        let mut render_pass = context
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui snapshot gpu-view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: context.target,
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
            slot.viewport[0],
            slot.viewport[1],
            slot.viewport[2].max(1.0),
            slot.viewport[3].max(1.0),
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(
            context.bounds.x,
            context.bounds.y,
            context.bounds.width,
            context.bounds.height,
        );
        render_pass.set_pipeline(&prepared.pipeline);
        render_pass.set_bind_group(0, &slot.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

struct PreparedGpuView {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    slots: HashMap<PrimitiveId, PreparedSlot>,
}

impl PreparedGpuView {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui snapshot gpu-view shader"),
            source: wgpu::ShaderSource::Wgsl(GPU_VIEW_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui snapshot gpu-view bind group layout"),
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
            label: Some("nana-ui snapshot gpu-view pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui snapshot gpu-view pipeline"),
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
            format,
            slots: HashMap::new(),
        }
    }
}

struct PreparedSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    viewport: [f32; 4],
}

#[repr(C)]
struct ViewUniform {
    color_a: [f32; 4],
    color_b: [f32; 4],
    parameters: [f32; 4],
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn color_rgba8(color: Color) -> [u8; 4] {
    [
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

fn uniform_bytes(uniform: &ViewUniform) -> [u8; 48] {
    let mut bytes = [0_u8; 48];
    let values = [
        uniform.color_a[0],
        uniform.color_a[1],
        uniform.color_a[2],
        uniform.color_a[3],
        uniform.color_b[0],
        uniform.color_b[1],
        uniform.color_b[2],
        uniform.color_b[3],
        uniform.parameters[0],
        uniform.parameters[1],
        uniform.parameters[2],
        uniform.parameters[3],
    ];
    for (index, value) in values.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
