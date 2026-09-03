use std::collections::{HashMap, VecDeque};

use bytemuck::{Pod, Zeroable};

use super::{
    clip::{self, FragmentClip, LogicalRect},
    color::{orthographic, pack_linear, with_opacity},
};
use crate::{PhysicalRect, icons::Icon};

const ATLAS_CAP: usize = 128;
const MAX_ATLAS_PX: u32 = 256;
const INITIAL_VERTICES: usize = 256;

const ICON_SHADER: &str = concat!(
    include_str!("shader/color.wgsl"),
    r#"
struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

@group(1) @binding(0)
var atlas: texture_2d<f32>;

@group(1) @binding(1)
var atlas_sampler: sampler;

struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec3<f32>,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec3<f32>,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    out.position = globals.transform * vec4<f32>(input.position, 0.0, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    out.world_pos = input.position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;
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
    let sampled = textureSample(atlas, atlas_sampler, input.uv);
    return vec4<f32>(sampled.rgb * input.color.rgb, sampled.a * input.color.a);
}
"#
);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct IconVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 3],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AtlasKey {
    icon: usize,
    px: u32,
}

struct AtlasSlot {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct FrameSlot {
    key: AtlasKey,
    first_vertex: u32,
    vertex_count: u32,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedIcon {
    index: usize,
}

pub(super) struct IconPipeline {
    pipeline: wgpu::RenderPipeline,
    atlas_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    pending_vertices: Vec<IconVertex>,
    frame_slots: Vec<FrameSlot>,
    atlas: HashMap<AtlasKey, AtlasSlot>,
    atlas_order: VecDeque<AtlasKey>,
}

impl IconPipeline {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.icon.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(ICON_SHADER)),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.icon.uniforms"),
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
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.icon.atlas"),
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
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.icon.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.icon.uniforms.bind"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.icon.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.icon.pipeline"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.icon.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<IconVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32x4,
                        4 => Float32x4,
                        5 => Float32x3,
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
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            atlas_layout,
            sampler,
            uniform_bind_group,
            uniforms,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.icon.vertices"),
                size: (INITIAL_VERTICES * std::mem::size_of::<IconVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_capacity: INITIAL_VERTICES,
            pending_vertices: Vec::new(),
            frame_slots: Vec::new(),
            atlas: HashMap::new(),
            atlas_order: VecDeque::new(),
        }
    }

    pub(super) fn begin_frame(&mut self, queue: &wgpu::Queue, physical_size: [u32; 2]) {
        self.pending_vertices.clear();
        self.frame_slots.clear();
        let uniforms = Uniforms {
            transform: orthographic(physical_size[0], physical_size[1]),
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: LogicalRect,
        affine: [f32; 6],
        persp: [f32; 2],
        scale: f32,
        icon: Icon,
        color: [f32; 4],
        opacity: f32,
        fragment_clip: FragmentClip,
    ) -> Option<PreparedIcon> {
        let extent = bounds.width.min(bounds.height);
        if extent <= 0.0 || scale <= 0.0 {
            return None;
        }
        let dest_px = (extent * scale).round().max(1.0) as u32;
        let px = dest_px.saturating_mul(2).clamp(2, MAX_ATLAS_PX);
        let key = AtlasKey {
            icon: icon.as_ptr() as usize,
            px,
        };
        if !self.atlas.contains_key(&key) {
            let rgba = rasterize_icon(icon.svg(), px)?;
            self.insert_atlas(device, queue, key, px, &rgba);
        }
        let color = pack_linear(with_opacity(color, opacity));
        let clip = fragment_clip.for_physical_pixels(scale);
        let first_vertex = self.pending_vertices.len() as u32;
        let [tl, tr, bl, br] = icon_quad(bounds, affine, persp, scale);
        let corners = [
            (tl, [0.0, 0.0]),
            (tr, [1.0, 0.0]),
            (bl, [0.0, 1.0]),
            (tr, [1.0, 0.0]),
            (br, [1.0, 1.0]),
            (bl, [0.0, 1.0]),
        ];
        for (position, uv) in corners {
            self.pending_vertices.push(IconVertex {
                position,
                uv,
                color,
                clip_rect: clip.rect,
                clip_inv_abcd: clip.inv_abcd,
                clip_inv_ef: [clip.inv_ef[0], clip.inv_ef[1], clip.corner_radius],
            });
        }
        let index = self.frame_slots.len();
        self.frame_slots.push(FrameSlot {
            key,
            first_vertex,
            vertex_count: 6,
        });
        Some(PreparedIcon { index })
    }

    pub(super) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.pending_vertices.is_empty() {
            return;
        }
        if self.pending_vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.pending_vertices.len().next_power_of_two();
            self.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.icon.vertices"),
                size: (self.vertex_capacity * std::mem::size_of::<IconVertex>()) as u64,
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

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedIcon,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(slot) = self.frame_slots.get(prepared.index) else {
            return;
        };
        let Some(atlas) = self.atlas.get(&slot.key) else {
            return;
        };
        if slot.vertex_count == 0 {
            return;
        }
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &atlas.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(
            slot.first_vertex..slot.first_vertex + slot.vertex_count,
            0..1,
        );
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }

    fn insert_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: AtlasKey,
        px: u32,
        rgba: &[u8],
    ) {
        while self.atlas.len() >= ATLAS_CAP {
            let Some(oldest) = self.atlas_order.pop_front() else {
                break;
            };
            if self.frame_slots.iter().any(|slot| slot.key == oldest) {
                self.atlas_order.push_back(oldest);
                break;
            }
            self.atlas.remove(&oldest);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui.scene.icon.atlas"),
            size: wgpu::Extent3d {
                width: px,
                height: px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(px * 4),
                rows_per_image: Some(px),
            },
            wgpu::Extent3d {
                width: px,
                height: px,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.icon.atlas.bind"),
            layout: &self.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.atlas_order.push_back(key);
        self.atlas.insert(
            key,
            AtlasSlot {
                _texture: texture,
                bind_group,
            },
        );
    }
}

fn icon_quad(bounds: LogicalRect, affine: [f32; 6], persp: [f32; 2], scale: f32) -> [[f32; 2]; 4] {
    let extent = bounds.width.min(bounds.height);
    let x = bounds.x + (bounds.width - extent) * 0.5;
    let y = bounds.y + (bounds.height - extent) * 0.5;
    if clip::is_translation_projective(affine, persp) {
        let [cx, cy] =
            clip::transform_point_projective(affine, persp, x + extent * 0.5, y + extent * 0.5);
        let (x0, px) = clip::snap_centered_origin(cx, extent, scale);
        let (y0, _) = clip::snap_centered_origin(cy, extent, scale);
        [[x0, y0], [x0 + px, y0], [x0, y0 + px], [x0 + px, y0 + px]]
    } else {
        [
            [x, y],
            [x + extent, y],
            [x, y + extent],
            [x + extent, y + extent],
        ]
        .map(|[px, py]| {
            let [tx, ty] = clip::transform_point_projective(affine, persp, px, py);
            [tx * scale, ty * scale]
        })
    }
}

fn rasterize_icon(svg: &str, pixel_size: u32) -> Option<Vec<u8>> {
    nana_svg_raster::rasterize_white_mask(svg, pixel_size, MAX_ATLAS_PX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_svg_rasterizes_coverage() {
        let rgba = rasterize_icon(Icon::Search.svg(), 28).expect("search svg");
        assert_eq!(rgba.len(), 28 * 28 * 4);
        let coverage = rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(
            coverage > 20,
            "search icon should ink the atlas, got {coverage}"
        );
        assert!(
            rgba.chunks(4).any(|pixel| pixel[3] < 16),
            "search icon should keep transparent padding"
        );
    }

    fn ink_bbox(rgba: &[u8], px: u32) -> (u32, u32, u32, u32) {
        let px = px as usize;
        let mut min_x = px;
        let mut min_y = px;
        let mut max_x = 0;
        let mut max_y = 0;
        for y in 0..px {
            for x in 0..px {
                if rgba[(y * px + x) * 4 + 3] > 16 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        (min_x as u32, min_y as u32, max_x as u32, max_y as u32)
    }

    #[test]
    fn symmetric_icons_are_centered_in_the_atlas() {
        let px = 48;
        for icon in [Icon::Add, Icon::Settings, Icon::Close] {
            let rgba = rasterize_icon(icon.svg(), px).expect("svg");
            let (min_x, min_y, max_x, max_y) = ink_bbox(&rgba, px);
            let cx = (min_x + max_x) as f32 / 2.0;
            let cy = (min_y + max_y) as f32 / 2.0;
            let mid = (px - 1) as f32 / 2.0;
            assert!(
                (cx - mid).abs() < 1.5,
                "{icon:?} horizontal center {cx} vs {mid}"
            );
            assert!(
                (cy - mid).abs() < 1.5,
                "{icon:?} vertical center {cy} vs {mid}"
            );
        }
    }
}
