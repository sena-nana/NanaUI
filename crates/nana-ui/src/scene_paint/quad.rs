use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use nana_ui_core::{
    BackgroundImage, BackgroundImageFit, BackgroundRepeat, BorderImageSpec, BorderImageTile,
    CssGradient, GradientStop, LengthSpec, LinearGradient, MAX_BACKGROUND_LAYERS,
};
use nana_ui_runtime::ComponentElevation;
use nana_ui_scene::QuadSurfacePaint;

use super::{
    clip::LogicalRect,
    color::{orthographic, pack_linear, with_opacity},
    image_url::decode_url_rgba,
};
use crate::PhysicalRect;

const INITIAL_INSTANCES: usize = 256;
const PAINT_GRADIENT: u32 = 1;
const PAINT_MASK: u32 = 2;
const PAINT_URL: u32 = 4;
const PAINT_FILTER: u32 = 8;
const PAINT_POLYGON: u32 = 16;
const PAINT_RADIAL: u32 = 32;
const PAINT_MASK_RADIAL: u32 = 64;
const PAINT_SHADOW_INSET: u32 = 128;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SolidInstance {
    color: [f32; 4],
    position: [f32; 2],
    size: [f32; 2],
    border_color: [f32; 4],
    border_radius: [f32; 4],
    border_widths: [f32; 4],
    shadow_color: [f32; 4],
    shadow_offset: [f32; 2],
    shadow_blur_radius: f32,
    shadow_spread_radius: f32,
    snap: u32,
    affine_abcd: [f32; 4],
    affine_ef: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct QuadPaintData {
    flags: u32,
    grad_angle: f32,
    grad_stop_count: u32,
    mask_stop_count: u32,
    mask_angle: f32,
    polygon_count: u32,
    url_tex_index: u32,
    url_fit: u32,
    filter_b: f32,
    filter_s: f32,
    filter_c: f32,
    filter_hue: f32,
    grad_stops0: [f32; 4],
    grad_stops1: [f32; 4],
    grad_stops2: [f32; 4],
    grad_stops3: [f32; 4],
    grad_pos: [f32; 4],
    grad_pos2: [f32; 4],
    mask_stops0: [f32; 4],
    mask_stops1: [f32; 4],
    mask_pos: [f32; 4],
    poly0: [f32; 4],
    poly1: [f32; 4],
    poly2: [f32; 4],
    poly3: [f32; 4],
    grad_stops4: [f32; 4],
    grad_stops5: [f32; 4],
    grad_stops6: [f32; 4],
    grad_stops7: [f32; 4],
    mask_stops2: [f32; 4],
    mask_stops3: [f32; 4],
    mask_stops4: [f32; 4],
    mask_stops5: [f32; 4],
    mask_stops6: [f32; 4],
    mask_stops7: [f32; 4],
    mask_pos2: [f32; 4],
    grad_center_x: f32,
    grad_center_y: f32,
    grad_radial_shape: u32,
    _pad_tail0: u32,
    mask_center_x: f32,
    mask_center_y: f32,
    mask_radial_shape: u32,
    _pad_tail1: u32,
    url_dest: [f32; 4],
    outline_width: f32,
    border_styles: u32,
    _pad_outline: [f32; 2],
    outline_color: [f32; 4],
    border_color_right: [f32; 4],
    border_color_bottom: [f32; 4],
    border_color_left: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<QuadPaintData>() == 560);
const _: () = assert!(std::mem::align_of::<QuadPaintData>() == 4);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
    scale: f32,
    _padding: [f32; 3],
}

struct CachedUrlTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

pub(super) struct QuadPipeline {
    pipeline: wgpu::RenderPipeline,
    pipeline_msaa: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    paint_buffer: wgpu::Buffer,
    paint_capacity: usize,
    url_view: wgpu::TextureView,
    url_sampler: wgpu::Sampler,
    url_cache: HashMap<String, Option<CachedUrlTexture>>,
    url_bind_groups: HashMap<Option<String>, wgpu::BindGroup>,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    pending: Vec<SolidInstance>,
    pending_paint: Vec<QuadPaintData>,
    pending_urls: Vec<Option<String>>,
}

impl QuadPipeline {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.quad.solid.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("shader/color.wgsl"),
                "\n",
                include_str!("shader/quad.wgsl"),
                "\n",
                include_str!("shader/vertex.wgsl"),
                "\n",
                include_str!("shader/quad_paint_data.wgsl"),
                "\n",
                include_str!("shader/quad_paint.wgsl"),
                "\n",
                include_str!("shader/quad_solid.wgsl"),
            ))),
        });
        let url_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.quad.url.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let url_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("nana-ui.scene.quad.url.fallback"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.quad.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Uniforms>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.quad.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let paint_capacity = INITIAL_INSTANCES;
        let paint_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.quad.paint"),
            size: (paint_capacity * std::mem::size_of::<QuadPaintData>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.quad.bind_group"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: paint_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&url_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&url_sampler),
                },
            ],
        });
        let mut url_bind_groups = HashMap::new();
        url_bind_groups.insert(None, bind_group);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.quad.solid.pipeline"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = solid_pipeline(device, &shader, &layout, format, 1);
        let pipeline_msaa = solid_pipeline(device, &shader, &layout, format, 4);
        let instance_capacity = INITIAL_INSTANCES;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.quad.instances"),
            size: (instance_capacity * std::mem::size_of::<SolidInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            pipeline_msaa,
            bind_layout,
            uniforms,
            paint_buffer,
            paint_capacity,
            url_view,
            url_sampler,
            url_cache: HashMap::new(),
            url_bind_groups,
            instances,
            instance_capacity,
            pending: Vec::new(),
            pending_paint: Vec::new(),
            pending_urls: Vec::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending.clear();
        self.pending_paint.clear();
        self.pending_urls.clear();
        self.url_bind_groups.clear();
    }

    pub(super) fn pending_len(&self) -> u32 {
        self.pending.len() as u32
    }

    pub(super) fn paint_buffer(&self) -> &wgpu::Buffer {
        &self.paint_buffer
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: LogicalRect,
        clip: LogicalRect,
        fragment_clip: super::clip::FragmentClip,
        affine: [f32; 6],
        persp: [f32; 2],
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        opacity: f32,
        surface: &QuadSurfacePaint,
    ) -> Option<u32> {
        let world = super::clip::transformed_aabb_projective(bounds, affine, persp);
        let _ = world.intersection(clip)?;
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return None;
        }
        let translation = super::clip::is_translation_projective(affine, persp);
        let (position, instance_affine, instance_persp, snap) = if translation {
            (
                [bounds.x + affine[4], bounds.y + affine[5]],
                super::clip::IDENTITY_AFFINE,
                [0.0, 0.0],
                1,
            )
        } else {
            ([bounds.x, bounds.y], affine, persp, 0)
        };
        let index = self.pending.len() as u32;
        let shadows = collect_shadows(shadow, surface);
        let split_shadows = shadows.len() > 1 || shadows.iter().any(|layer| layer.inset);
        let outsets = shadows
            .iter()
            .copied()
            .filter(|layer| !layer.inset)
            .collect::<Vec<_>>();
        let insets = shadows
            .iter()
            .copied()
            .filter(|layer| layer.inset)
            .collect::<Vec<_>>();
        let fill_shadow = if split_shadows {
            None
        } else {
            outsets.first().copied()
        };
        let (mut edge_widths, edge_colors) =
            resolved_instance_border(border_color, border_width, surface);
        let zero_widths = [0.0f32; 4];
        let zero_colors = [[0.0f32; 4]; 4];
        let mut layers = surface_paint_layers(surface);
        let content_layer = if surface.content_image.is_some() {
            layers.pop()
        } else {
            None
        };
        let border_tiles = prepare_border_image_tiles(
            device,
            queue,
            &mut self.url_cache,
            surface.border_image.as_ref(),
            bounds.width,
            bounds.height,
        );
        if border_tiles.is_some() {
            edge_widths = zero_widths;
        }
        if split_shadows {
            for layer in outsets.iter().rev() {
                push_solid_instance(
                    &mut self.pending,
                    &mut self.pending_paint,
                    &mut self.pending_urls,
                    shadow_only_paint(false),
                    None,
                    position,
                    [bounds.width, bounds.height],
                    None,
                    zero_colors,
                    zero_widths,
                    corner_radius,
                    Some(*layer),
                    opacity,
                    snap,
                    instance_affine,
                    instance_persp,
                    fragment_clip,
                );
            }
        }
        if layers.is_empty() {
            let paint = pack_shared(
                device,
                queue,
                &mut self.url_cache,
                surface,
                bounds.width,
                bounds.height,
                None,
            );
            push_solid_instance(
                &mut self.pending,
                &mut self.pending_paint,
                &mut self.pending_urls,
                paint,
                None,
                position,
                [bounds.width, bounds.height],
                background,
                edge_colors,
                edge_widths,
                corner_radius,
                fill_shadow,
                opacity,
                snap,
                instance_affine,
                instance_persp,
                fragment_clip,
            );
        } else {
            for (layer_index, layer) in layers.iter().enumerate() {
                let (paint, paint_url) = pack_layer(
                    device,
                    queue,
                    &mut self.url_cache,
                    surface,
                    layer,
                    bounds.width,
                    bounds.height,
                );
                let first = layer_index == 0;
                push_solid_instance(
                    &mut self.pending,
                    &mut self.pending_paint,
                    &mut self.pending_urls,
                    paint,
                    paint_url,
                    position,
                    [bounds.width, bounds.height],
                    if first { background } else { None },
                    if first { edge_colors } else { zero_colors },
                    if first { edge_widths } else { zero_widths },
                    corner_radius,
                    if first { fill_shadow } else { None },
                    opacity,
                    snap,
                    instance_affine,
                    instance_persp,
                    fragment_clip,
                );
            }
        }
        if let Some((paint_url, tiles)) = border_tiles.as_ref() {
            for tile in tiles {
                let paint = QuadPaintData {
                    flags: PAINT_URL,
                    url_dest: url_dest_for_uv(tile.u0, tile.v0, tile.u1, tile.v1),
                    ..Default::default()
                };
                push_solid_instance(
                    &mut self.pending,
                    &mut self.pending_paint,
                    &mut self.pending_urls,
                    paint,
                    Some(paint_url.clone()),
                    [position[0] + tile.dest_x, position[1] + tile.dest_y],
                    [tile.dest_w, tile.dest_h],
                    None,
                    zero_colors,
                    zero_widths,
                    [0.0; 4],
                    None,
                    opacity,
                    snap,
                    instance_affine,
                    instance_persp,
                    fragment_clip,
                );
            }
        }
        if let Some(layer) = content_layer {
            let (paint, paint_url) = pack_layer(
                device,
                queue,
                &mut self.url_cache,
                surface,
                layer,
                bounds.width,
                bounds.height,
            );
            push_solid_instance(
                &mut self.pending,
                &mut self.pending_paint,
                &mut self.pending_urls,
                paint,
                paint_url,
                position,
                [bounds.width, bounds.height],
                None,
                zero_colors,
                zero_widths,
                corner_radius,
                None,
                opacity,
                snap,
                instance_affine,
                instance_persp,
                fragment_clip,
            );
        }
        if split_shadows {
            for layer in insets.iter().rev() {
                push_solid_instance(
                    &mut self.pending,
                    &mut self.pending_paint,
                    &mut self.pending_urls,
                    shadow_only_paint(true),
                    None,
                    position,
                    [bounds.width, bounds.height],
                    None,
                    zero_colors,
                    zero_widths,
                    corner_radius,
                    Some(*layer),
                    opacity,
                    snap,
                    instance_affine,
                    instance_persp,
                    fragment_clip,
                );
            }
        }
        Some(index)
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
            transform: orthographic(physical_size[0], physical_size[1]),
            scale: scale_factor,
            _padding: [0.0; 3],
        };
        let uniform_bytes = bytemuck::bytes_of(&uniforms);
        queue.write_buffer(&self.uniforms, 0, uniform_bytes);
        if let Some(work) = gpu_work {
            work.record_upload(uniform_bytes.len());
        }
        if self.pending.is_empty() {
            return;
        }
        if self.pending.len() > self.instance_capacity {
            self.instance_capacity = self.pending.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.quad.instances"),
                size: (self.instance_capacity * std::mem::size_of::<SolidInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(work) = gpu_work {
                work.record_realloc();
            }
        }
        let paint_reallocated = self.pending_paint.len() > self.paint_capacity;
        if paint_reallocated {
            self.paint_capacity = self.pending_paint.len().next_power_of_two();
            self.paint_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui.scene.quad.paint"),
                size: (self.paint_capacity * std::mem::size_of::<QuadPaintData>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(work) = gpu_work {
                work.record_realloc();
            }
        }
        let upload_bytes = {
            let instance_bytes = bytemuck::cast_slice(&self.pending);
            queue.write_buffer(&self.instances, 0, instance_bytes);
            let paint_bytes = bytemuck::cast_slice(&self.pending_paint);
            queue.write_buffer(&self.paint_buffer, 0, paint_bytes);
            instance_bytes.len() + paint_bytes.len()
        };
        self.rebuild_url_bind_groups(device);
        if let Some(work) = gpu_work {
            work.record_upload(upload_bytes);
            work.record_batch_rebuild();
        }
    }

    fn rebuild_url_bind_groups(&mut self, device: &wgpu::Device) {
        self.url_bind_groups.clear();
        let mut unique = Vec::new();
        for url in &self.pending_urls {
            if !unique.iter().any(|existing| existing == url) {
                unique.push(url.clone());
            }
        }
        if !unique.iter().any(|url| url.is_none()) {
            unique.push(None);
        }
        for url in unique {
            let bind_group = self.create_url_bind_group(device, url.as_deref());
            self.url_bind_groups.insert(url, bind_group);
        }
    }

    fn create_url_bind_group(&self, device: &wgpu::Device, url: Option<&str>) -> wgpu::BindGroup {
        let url_view = url
            .and_then(|key| self.url_cache.get(key))
            .and_then(|cached| cached.as_ref())
            .map(|cached| &cached.view)
            .unwrap_or(&self.url_view);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.quad.bind_group"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.paint_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(url_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.url_sampler),
                },
            ],
        })
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: std::ops::Range<u32>,
        scissor: PhysicalRect,
        sample_count: u32,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        if range.start >= range.end {
            return;
        }
        pass.set_pipeline(if sample_count > 1 {
            &self.pipeline_msaa
        } else {
            &self.pipeline
        });
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let start = range.start as usize;
        let end = range.end as usize;
        let mut index = start;
        while index < end {
            let url = self.pending_urls.get(index).cloned().unwrap_or(None);
            let batch_start = index;
            index += 1;
            while index < end && self.pending_urls.get(index).cloned().unwrap_or(None) == url {
                index += 1;
            }
            let bind_group = self
                .url_bind_groups
                .get(&url)
                .or_else(|| self.url_bind_groups.get(&None))
                .expect("quad url bind groups must be rebuilt in upload");
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, batch_start as u32..index as u32);
            if let Some(work) = gpu_work {
                work.record_draw_batch();
                work.record_draw_call();
            }
        }
    }
}

fn collect_shadows(
    primary: Option<ComponentElevation>,
    surface: &QuadSurfacePaint,
) -> Vec<ComponentElevation> {
    let mut shadows = Vec::new();
    if let Some(shadow) = primary {
        shadows.push(shadow);
    }
    shadows.extend(surface.extra_shadows.iter().copied());
    shadows.truncate(nana_ui_core::MAX_BOX_SHADOWS);
    shadows
}

fn shadow_only_paint(inset: bool) -> QuadPaintData {
    let mut paint = QuadPaintData::default();
    if inset {
        paint.flags |= PAINT_SHADOW_INSET;
    }
    paint
}

fn push_solid_instance(
    pending: &mut Vec<SolidInstance>,
    pending_paint: &mut Vec<QuadPaintData>,
    pending_urls: &mut Vec<Option<String>>,
    mut paint: QuadPaintData,
    paint_url: Option<String>,
    position: [f32; 2],
    size: [f32; 2],
    background: Option<[f32; 4]>,
    border_colors: [[f32; 4]; 4],
    border_widths: [f32; 4],
    corner_radius: [f32; 4],
    shadow: Option<ComponentElevation>,
    opacity: f32,
    snap: u32,
    instance_affine: [f32; 6],
    instance_persp: [f32; 2],
    fragment_clip: super::clip::FragmentClip,
) {
    paint.border_color_right = pack_linear(with_opacity(border_colors[1], opacity));
    paint.border_color_bottom = pack_linear(with_opacity(border_colors[2], opacity));
    paint.border_color_left = pack_linear(with_opacity(border_colors[3], opacity));
    pending_paint.push(paint);
    pending_urls.push(paint_url);
    pending.push(SolidInstance {
        color: pack_linear(with_opacity(
            background.unwrap_or([0.0, 0.0, 0.0, 0.0]),
            opacity,
        )),
        position,
        size,
        border_color: pack_linear(with_opacity(border_colors[0], opacity)),
        border_radius: corner_radius,
        border_widths,
        shadow_color: pack_linear(with_opacity(
            shadow
                .map(|shadow| shadow.color)
                .unwrap_or([0.0, 0.0, 0.0, 0.0]),
            opacity,
        )),
        shadow_offset: [
            shadow.map(|shadow| shadow.offset_x).unwrap_or(0.0),
            shadow.map(|shadow| shadow.offset_y).unwrap_or(0.0),
        ],
        shadow_blur_radius: shadow.map(|shadow| shadow.blur_radius).unwrap_or(0.0),
        shadow_spread_radius: shadow.map(|shadow| shadow.spread_radius).unwrap_or(0.0)
            + paint.outline_width.max(0.0),
        snap,
        affine_abcd: [
            instance_affine[0],
            instance_affine[1],
            instance_affine[2],
            instance_affine[3],
        ],
        affine_ef: [
            instance_affine[4],
            instance_affine[5],
            instance_persp[0],
            instance_persp[1],
        ],
        clip_rect: fragment_clip.rect,
        clip_inv_abcd: fragment_clip.inv_abcd,
        clip_inv_ef: [
            fragment_clip.inv_ef[0],
            fragment_clip.inv_ef[1],
            fragment_clip.corner_radius,
        ],
    });
}

fn pack_border_styles(codes: [u8; 4]) -> u32 {
    (u32::from(codes[0]) & 3)
        | ((u32::from(codes[1]) & 3) << 2)
        | ((u32::from(codes[2]) & 3) << 4)
        | ((u32::from(codes[3]) & 3) << 6)
}

fn resolved_instance_border(
    border_color: Option<[f32; 4]>,
    border_width: f32,
    surface: &QuadSurfacePaint,
) -> ([f32; 4], [[f32; 4]; 4]) {
    let fallback = border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    if surface
        .border_widths
        .iter()
        .copied()
        .any(|width| width > 0.0)
    {
        let mut colors = surface.border_colors;
        for color in colors.iter_mut() {
            if color[3] <= 0.0 {
                *color = fallback;
            }
        }
        (surface.border_widths, colors)
    } else {
        ([border_width.max(0.0); 4], [fallback; 4])
    }
}

fn surface_paint_layers(surface: &QuadSurfacePaint) -> Vec<&BackgroundImage> {
    let mut layers = Vec::new();
    for layer in surface.background_layers.iter().rev() {
        layers.push(layer);
    }
    if let Some(image) = surface.background_image.as_ref() {
        layers.push(image);
    }
    let cap = if surface.content_image.is_some() {
        MAX_BACKGROUND_LAYERS.saturating_sub(1)
    } else {
        MAX_BACKGROUND_LAYERS
    };
    if layers.len() > cap {
        let skip = layers.len() - cap;
        layers.drain(..skip);
    }
    if let Some(image) = surface.content_image.as_ref() {
        layers.push(image);
    }
    layers
}

fn pack_paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    surface: &QuadSurfacePaint,
    width: f32,
    height: f32,
) -> QuadPaintData {
    let layers = surface_paint_layers(surface);
    pack_shared(
        device,
        queue,
        cache,
        surface,
        width,
        height,
        layers.first().copied(),
    )
}

fn pack_layer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    surface: &QuadSurfacePaint,
    layer: &BackgroundImage,
    width: f32,
    height: f32,
) -> (QuadPaintData, Option<String>) {
    let paint = pack_shared(device, queue, cache, surface, width, height, Some(layer));
    let paint_url = if paint.flags & PAINT_URL != 0 {
        layer.url_str().map(str::to_string)
    } else {
        None
    };
    (paint, paint_url)
}

fn pack_shared(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    surface: &QuadSurfacePaint,
    width: f32,
    height: f32,
    layer: Option<&BackgroundImage>,
) -> QuadPaintData {
    let mut paint = QuadPaintData::default();
    if let Some(BackgroundImage::Gradient(grad)) = layer {
        paint.flags |= PAINT_GRADIENT;
        match grad {
            CssGradient::Linear(linear) => {
                paint.grad_angle = linear.angle_deg;
                pack_gradient_stops(&mut paint, &linear.stops);
            }
            CssGradient::Radial(radial) => {
                paint.flags |= PAINT_RADIAL;
                paint.grad_center_x = radial.center[0];
                paint.grad_center_y = radial.center[1];
                paint.grad_radial_shape = if radial.circle { 0 } else { 1 };
                pack_gradient_stops(&mut paint, &radial.stops);
            }
        }
    }
    if let Some(mask) = surface.mask.as_ref() {
        paint.flags |= PAINT_MASK;
        match mask {
            CssGradient::Linear(linear) => {
                paint.mask_angle = linear.angle_deg;
                pack_mask_stops(&mut paint, &linear.stops);
            }
            CssGradient::Radial(radial) => {
                paint.flags |= PAINT_MASK_RADIAL;
                paint.mask_center_x = radial.center[0];
                paint.mask_center_y = radial.center[1];
                paint.mask_radial_shape = if radial.circle { 0 } else { 1 };
                pack_mask_stops(&mut paint, &radial.stops);
            }
        }
    }
    if let Some(filter) = surface.filter {
        if !filter.is_identity() {
            paint.flags |= PAINT_FILTER;
            paint.filter_b = filter.brightness;
            paint.filter_s = filter.saturate;
            paint.filter_c = filter.contrast;
            paint.filter_hue = filter.hue_rotate_deg;
        }
    }
    if surface.outline_width > 0.0 {
        paint.outline_width = surface.outline_width;
        paint.outline_color = surface.outline_color.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    }
    paint.border_styles = pack_border_styles(surface.border_styles);
    if let Some(points) = surface.polygon_clip.as_ref() {
        if points.len() >= 3 {
            paint.flags |= PAINT_POLYGON;
            paint.polygon_count = points.len().min(8) as u32;
            let mut flat = [[0.0f32; 2]; 8];
            for (index, point) in points.iter().take(8).enumerate() {
                flat[index] = [point[0] / width.max(1.0), point[1] / height.max(1.0)];
            }
            paint.poly0 = [flat[0][0], flat[0][1], flat[1][0], flat[1][1]];
            paint.poly1 = [flat[2][0], flat[2][1], flat[3][0], flat[3][1]];
            paint.poly2 = [flat[4][0], flat[4][1], flat[5][0], flat[5][1]];
            paint.poly3 = [flat[6][0], flat[6][1], flat[7][0], flat[7][1]];
        }
    }
    if let Some(BackgroundImage::Url {
        url,
        fit,
        size_width,
        size_height,
        position,
        repeat,
    }) = layer
    {
        if let Some((tex_w, tex_h)) = load_url_texture(device, queue, cache, url) {
            paint.flags |= PAINT_URL;
            paint.url_tex_index = repeat_bits(*repeat);
            paint.url_fit = match fit {
                BackgroundImageFit::Cover => 0,
                BackgroundImageFit::Contain => 1,
                BackgroundImageFit::Stretch => 2,
                BackgroundImageFit::Auto => 3,
                BackgroundImageFit::Length => 4,
            };
            paint.url_dest = url_dest_rect(
                *fit,
                *size_width,
                *size_height,
                *position,
                width,
                height,
                tex_w as f32,
                tex_h as f32,
            );
        }
    }
    paint
}

fn pack_stop_arrays(stops: &[nana_ui_core::GradientStop]) -> (u32, [[f32; 4]; 8], [f32; 8]) {
    let count = stops.len().min(8) as u32;
    let mut colors = [[0.0; 4]; 8];
    let mut positions = [0.0; 8];
    for (index, stop) in stops.iter().take(8).enumerate() {
        colors[index] = stop.color;
        positions[index] = stop.position;
    }
    (count, colors, positions)
}

fn pack_gradient_stops(paint: &mut QuadPaintData, stops: &[nana_ui_core::GradientStop]) {
    let (count, colors, positions) = pack_stop_arrays(stops);
    paint.grad_stop_count = count;
    paint.grad_stops0 = colors[0];
    paint.grad_stops1 = colors[1];
    paint.grad_stops2 = colors[2];
    paint.grad_stops3 = colors[3];
    paint.grad_stops4 = colors[4];
    paint.grad_stops5 = colors[5];
    paint.grad_stops6 = colors[6];
    paint.grad_stops7 = colors[7];
    paint.grad_pos = [positions[0], positions[1], positions[2], positions[3]];
    paint.grad_pos2 = [positions[4], positions[5], positions[6], positions[7]];
}

fn pack_mask_stops(paint: &mut QuadPaintData, stops: &[nana_ui_core::GradientStop]) {
    let (count, colors, positions) = pack_stop_arrays(stops);
    paint.mask_stop_count = count;
    paint.mask_stops0 = colors[0];
    paint.mask_stops1 = colors[1];
    paint.mask_stops2 = colors[2];
    paint.mask_stops3 = colors[3];
    paint.mask_stops4 = colors[4];
    paint.mask_stops5 = colors[5];
    paint.mask_stops6 = colors[6];
    paint.mask_stops7 = colors[7];
    paint.mask_pos = [positions[0], positions[1], positions[2], positions[3]];
    paint.mask_pos2 = [positions[4], positions[5], positions[6], positions[7]];
}

fn load_url_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    url: &str,
) -> Option<(u32, u32)> {
    if let Some(cached) = cache.get(url) {
        return cached.as_ref().map(|entry| (entry.width, entry.height));
    }
    let Some((width, height, rgba)) = decode_url_rgba(url) else {
        cache.insert(url.to_string(), None);
        return None;
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui.scene.quad.url"),
        size: wgpu::Extent3d {
            width,
            height,
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
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    cache.insert(
        url.to_string(),
        Some(CachedUrlTexture {
            _texture: texture,
            view,
            width,
            height,
        }),
    );
    Some((width, height))
}

const BORDER_IMAGE_LINEAR_SIZE: u32 = 64;

fn prepare_border_image_tiles(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    spec: Option<&BorderImageSpec>,
    box_w: f32,
    box_h: f32,
) -> Option<(String, Vec<BorderImageTile>)> {
    let spec = spec.filter(|spec| spec.paints_linear_or_url())?;
    let (key, image_w, image_h) = match &spec.source {
        BackgroundImage::Url { url, .. } => {
            if url.is_empty() {
                return None;
            }
            let (tex_w, tex_h) = load_url_texture(device, queue, cache, url)?;
            (url.clone(), tex_w as f32, tex_h as f32)
        }
        BackgroundImage::Gradient(CssGradient::Linear(linear)) => {
            let key = linear_gradient_cache_key(linear);
            if !cache.contains_key(&key) {
                let rgba = rasterize_linear_gradient(linear, BORDER_IMAGE_LINEAR_SIZE);
                insert_rgba_texture(
                    device,
                    queue,
                    cache,
                    &key,
                    BORDER_IMAGE_LINEAR_SIZE,
                    BORDER_IMAGE_LINEAR_SIZE,
                    &rgba,
                );
            } else if cache.get(&key).is_some_and(Option::is_none) {
                return None;
            }
            (key, box_w, box_h)
        }
        BackgroundImage::Gradient(CssGradient::Radial(_)) => return None,
    };
    let tiles = spec.tiles(image_w, image_h, box_w, box_h);
    if tiles.is_empty() {
        None
    } else {
        Some((key, tiles))
    }
}

fn insert_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, Option<CachedUrlTexture>>,
    key: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui.scene.quad.border-image"),
        size: wgpu::Extent3d {
            width,
            height,
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
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    cache.insert(
        key.to_string(),
        Some(CachedUrlTexture {
            _texture: texture,
            view,
            width,
            height,
        }),
    );
}

fn linear_gradient_cache_key(linear: &LinearGradient) -> String {
    let mut key = format!("nana:border-image-linear:{:.4}", linear.angle_deg);
    for stop in linear.stops.iter().take(8) {
        key.push_str(&format!(
            ":{:.4},{:.4},{:.4},{:.4},{:.4}",
            stop.position, stop.color[0], stop.color[1], stop.color[2], stop.color[3]
        ));
    }
    key
}

fn rasterize_linear_gradient(linear: &LinearGradient, size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let size_f = size as f32;
    for y in 0..size {
        for x in 0..size {
            let t = cpu_gradient_t(
                (x as f32 + 0.5) / size_f,
                (y as f32 + 0.5) / size_f,
                linear.angle_deg,
            );
            let color = cpu_sample_stops(t, &linear.stops);
            let i = ((y * size + x) * 4) as usize;
            rgba[i] = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 1] = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 2] = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i + 3] = (color[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    rgba
}

fn cpu_gradient_t(lx: f32, ly: f32, angle_deg: f32) -> f32 {
    let rad = angle_deg * 0.017453292;
    let axis_x = rad.sin();
    let axis_y = -rad.cos();
    let denom = axis_x.abs() + axis_y.abs();
    if denom <= 0.0001 {
        return 0.5;
    }
    ((lx - 0.5) * axis_x / denom + (ly - 0.5) * axis_y / denom + 0.5).clamp(0.0, 1.0)
}

fn cpu_sample_stops(t: f32, stops: &[GradientStop]) -> [f32; 4] {
    if stops.is_empty() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    if stops.len() == 1 || t <= stops[0].position {
        return stops[0].color;
    }
    if t >= stops[stops.len() - 1].position {
        return stops[stops.len() - 1].color;
    }
    for window in stops.windows(2) {
        let a = window[0];
        let b = window[1];
        if t >= a.position && t <= b.position {
            let mix = (t - a.position) / (b.position - a.position).max(0.0001);
            return [
                a.color[0] + (b.color[0] - a.color[0]) * mix,
                a.color[1] + (b.color[1] - a.color[1]) * mix,
                a.color[2] + (b.color[2] - a.color[2]) * mix,
                a.color[3] + (b.color[3] - a.color[3]) * mix,
            ];
        }
    }
    stops[0].color
}

fn url_dest_for_uv(u0: f32, v0: f32, u1: f32, v1: f32) -> [f32; 4] {
    let du = (u1 - u0).max(1.0e-6);
    let dv = (v1 - v0).max(1.0e-6);
    [-u0 / du, -v0 / dv, 1.0 / du, 1.0 / dv]
}

fn repeat_bits(repeat: BackgroundRepeat) -> u32 {
    match repeat {
        BackgroundRepeat::NoRepeat => 0,
        BackgroundRepeat::Repeat => 1 | 2,
        BackgroundRepeat::RepeatX => 1,
        BackgroundRepeat::RepeatY => 2,
    }
}

fn url_dest_rect(
    fit: BackgroundImageFit,
    size_width: Option<LengthSpec>,
    size_height: Option<LengthSpec>,
    position: nana_ui_core::BackgroundPosition,
    box_w: f32,
    box_h: f32,
    tex_w: f32,
    tex_h: f32,
) -> [f32; 4] {
    let (drawn_w, drawn_h) = match fit {
        BackgroundImageFit::Stretch => (box_w, box_h),
        BackgroundImageFit::Cover => {
            let scale = (box_w / tex_w.max(1.0)).max(box_h / tex_h.max(1.0));
            (tex_w * scale, tex_h * scale)
        }
        BackgroundImageFit::Contain => {
            let scale = (box_w / tex_w.max(1.0)).min(box_h / tex_h.max(1.0));
            (tex_w * scale, tex_h * scale)
        }
        BackgroundImageFit::Auto => (tex_w, tex_h),
        BackgroundImageFit::Length => {
            resolve_explicit_size(size_width, size_height, box_w, box_h, tex_w, tex_h)
        }
    };
    let origin_x = position_origin(position.x, box_w, drawn_w);
    let origin_y = position_origin(position.y, box_h, drawn_h);
    [
        origin_x / box_w.max(1.0),
        origin_y / box_h.max(1.0),
        drawn_w / box_w.max(1.0),
        drawn_h / box_h.max(1.0),
    ]
}

fn resolve_explicit_size(
    size_width: Option<LengthSpec>,
    size_height: Option<LengthSpec>,
    box_w: f32,
    box_h: f32,
    tex_w: f32,
    tex_h: f32,
) -> (f32, f32) {
    let width = size_width.and_then(|spec| resolve_size_len(spec, box_w));
    let height = size_height.and_then(|spec| resolve_size_len(spec, box_h));
    match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, width * tex_h / tex_w.max(1.0)),
        (None, Some(height)) => (height * tex_w / tex_h.max(1.0), height),
        (None, None) => (tex_w, tex_h),
    }
}

fn resolve_size_len(spec: LengthSpec, box_len: f32) -> Option<f32> {
    match spec {
        LengthSpec::Auto | LengthSpec::Fill | LengthSpec::Shrink => None,
        other => other.resolve_px(Some(box_len)),
    }
}

fn position_origin(spec: LengthSpec, box_len: f32, image_len: f32) -> f32 {
    match spec {
        LengthSpec::Percent(percent) => (box_len - image_len) * (percent / 100.0),
        LengthSpec::Px(value) => value,
        other => other.resolve_px(Some(box_len)).unwrap_or(0.0),
    }
}

fn solid_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("nana-ui.scene.quad.solid.pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("solid_vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SolidInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array!(
                    0 => Float32x4,
                    1 => Float32x2,
                    2 => Float32x2,
                    3 => Float32x4,
                    4 => Float32x4,
                    5 => Float32x4,
                    6 => Float32x4,
                    7 => Float32x2,
                    8 => Float32,
                    9 => Float32,
                    10 => Uint32,
                    11 => Float32x4,
                    12 => Float32x4,
                    13 => Float32x4,
                    14 => Float32x4,
                    15 => Float32x3,
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

#[cfg(test)]
fn quad_paint_test_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("quad paint test requires a WGPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-ui quad paint test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("quad paint test requires a WGPU device")
}

#[cfg(test)]
#[test]
fn pack_paint_sets_mask_flag() {
    use nana_ui_core::{CssGradient, GradientStop, LinearGradient};
    use nana_ui_scene::QuadSurfacePaint;

    let (device, queue) = quad_paint_test_device();
    let surface = QuadSurfacePaint {
        mask: Some(CssGradient::Linear(LinearGradient {
            angle_deg: 90.0,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: [1.0, 1.0, 1.0, 1.0],
                },
                GradientStop {
                    position: 1.0,
                    color: [1.0, 1.0, 1.0, 0.0],
                },
            ],
        })),
        ..Default::default()
    };
    let paint = pack_paint(&device, &queue, &mut HashMap::new(), &surface, 64.0, 64.0);
    assert_ne!(paint.flags & PAINT_MASK, 0, "flags={}", paint.flags);
    assert_eq!(paint.mask_stop_count, 2);
}

#[cfg(test)]
#[test]
fn pack_paint_sets_hue_rotate() {
    use nana_ui_core::ColorFilter;
    use nana_ui_scene::QuadSurfacePaint;

    let (device, queue) = quad_paint_test_device();
    let surface = QuadSurfacePaint {
        filter: Some(ColorFilter {
            hue_rotate_deg: 90.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    let paint = pack_paint(&device, &queue, &mut HashMap::new(), &surface, 64.0, 64.0);
    assert_ne!(paint.flags & PAINT_FILTER, 0, "flags={}", paint.flags);
    assert!((paint.filter_hue - 90.0).abs() < 0.01);
}

#[cfg(test)]
#[test]
fn pack_paint_packs_dashed_border_styles() {
    use nana_ui_scene::QuadSurfacePaint;

    let (device, queue) = quad_paint_test_device();
    let surface = QuadSurfacePaint {
        border_styles: [1, 2, 1, 0],
        ..Default::default()
    };
    let paint = pack_paint(&device, &queue, &mut HashMap::new(), &surface, 64.0, 64.0);
    assert_eq!(paint.border_styles, 1 | (2 << 2) | (1 << 4));
}

#[cfg(test)]
#[test]
fn url_dest_for_uv_maps_source_slice() {
    let dest = url_dest_for_uv(0.25, 0.0, 0.75, 0.5);
    let uv0_x = (0.0 - dest[0]) / dest[2];
    let uv1_x = (1.0 - dest[0]) / dest[2];
    let uv0_y = (0.0 - dest[1]) / dest[3];
    let uv1_y = (1.0 - dest[1]) / dest[3];
    assert!((uv0_x - 0.25).abs() < 1.0e-5);
    assert!((uv1_x - 0.75).abs() < 1.0e-5);
    assert!((uv0_y - 0.0).abs() < 1.0e-5);
    assert!((uv1_y - 0.5).abs() < 1.0e-5);
}
