use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use nana_ui_core::{BackgroundImage, BackgroundImageFit, CssGradient};
use nana_ui_runtime::ComponentElevation;
use nana_ui_scene::QuadSurfacePaint;

use super::{
    clip::LogicalRect,
    color::{orthographic, pack_linear, with_opacity},
    image_url::resolve_background_image_url,
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SolidInstance {
    color: [f32; 4],
    position: [f32; 2],
    size: [f32; 2],
    border_color: [f32; 4],
    border_radius: [f32; 4],
    border_width: f32,
    shadow_color: [f32; 4],
    shadow_offset: [f32; 2],
    shadow_blur_radius: f32,
    shadow_spread_radius: f32,
    snap: u32,
    affine_abcd: [f32; 4],
    affine_ef: [f32; 2],
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
    _pad_filter: [f32; 1],
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
}

const _: () = assert!(std::mem::size_of::<QuadPaintData>() == 464);
const _: () = assert!(std::mem::align_of::<QuadPaintData>() == 4);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    transform: [f32; 16],
    scale: f32,
    _padding: [f32; 3],
}

struct CachedUrlTexture {
    view: wgpu::TextureView,
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
    url_cache: HashMap<String, CachedUrlTexture>,
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
                    visibility: wgpu::ShaderStages::VERTEX,
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
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        opacity: f32,
        surface: &QuadSurfacePaint,
    ) -> Option<u32> {
        let world = super::clip::transformed_aabb(bounds, affine);
        let _ = world.intersection(clip)?;
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return None;
        }
        let translation = super::clip::is_translation(affine);
        let (position, instance_affine, snap) = if translation {
            (
                [bounds.x + affine[4], bounds.y + affine[5]],
                super::clip::IDENTITY_AFFINE,
                1,
            )
        } else {
            ([bounds.x, bounds.y], affine, 0)
        };
        let index = self.pending.len() as u32;
        let paint = pack_paint(
            device,
            queue,
            &mut self.url_cache,
            surface,
            bounds.width,
            bounds.height,
        );
        let paint_url = if paint.flags & PAINT_URL != 0 {
            surface
                .background_image
                .as_ref()
                .and_then(|image| match image {
                    BackgroundImage::Url { url, .. } => Some(url.clone()),
                    _ => None,
                })
        } else {
            None
        };
        self.pending_paint.push(paint);
        self.pending_urls.push(paint_url);
        self.pending.push(SolidInstance {
            color: pack_linear(with_opacity(
                background.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                opacity,
            )),
            position,
            size: [bounds.width, bounds.height],
            border_color: pack_linear(with_opacity(
                border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                opacity,
            )),
            border_radius: corner_radius,
            border_width,
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
            shadow_spread_radius: shadow.map(|shadow| shadow.spread_radius).unwrap_or(0.0),
            snap,
            affine_abcd: [
                instance_affine[0],
                instance_affine[1],
                instance_affine[2],
                instance_affine[3],
            ],
            affine_ef: [instance_affine[4], instance_affine[5]],
            clip_rect: fragment_clip.rect,
            clip_inv_abcd: fragment_clip.inv_abcd,
            clip_inv_ef: [
                fragment_clip.inv_ef[0],
                fragment_clip.inv_ef[1],
                fragment_clip.corner_radius,
            ],
        });
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

fn pack_paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &mut HashMap<String, CachedUrlTexture>,
    surface: &QuadSurfacePaint,
    width: f32,
    height: f32,
) -> QuadPaintData {
    let mut paint = QuadPaintData::default();
    if let Some(BackgroundImage::Gradient(grad)) = surface.background_image.as_ref() {
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
        }
    }
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
    if let Some(BackgroundImage::Url { url, fit }) = surface.background_image.as_ref() {
        if load_url_texture(device, queue, cache, url) {
            paint.flags |= PAINT_URL;
            paint.url_tex_index = 0;
            paint.url_fit = match fit {
                BackgroundImageFit::Cover => 0,
                BackgroundImageFit::Contain => 1,
                BackgroundImageFit::Stretch => 2,
            };
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
    cache: &mut HashMap<String, CachedUrlTexture>,
    url: &str,
) -> bool {
    if cache.contains_key(url) {
        return true;
    }
    let Some((width, height, rgba)) = decode_url_rgba(url) else {
        return false;
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
    cache.insert(url.to_string(), CachedUrlTexture { view });
    true
}

fn decode_url_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let resolved = resolve_background_image_url(url)?;
    if resolved.starts_with("data:") {
        return decode_data_url_rgba(&resolved);
    }
    if resolved.starts_with("http://") || resolved.starts_with("https://") {
        return decode_http_rgba(&resolved);
    }
    let bytes = std::fs::read(&resolved).ok()?;
    decode_image_bytes(&bytes)
}

fn decode_data_url_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let payload = url
        .strip_prefix("data:image/png;base64,")
        .or_else(|| url.strip_prefix("data:image/jpeg;base64,"))?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    decode_image_bytes(&bytes)
}

fn decode_http_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let mut response = ureq::get(url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.body_mut().read_to_vec().ok()?;
    decode_image_bytes(&bytes)
}

fn decode_image_bytes(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((width, height, rgba.into_raw()))
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
                    5 => Float32,
                    6 => Float32x4,
                    7 => Float32x2,
                    8 => Float32,
                    9 => Float32,
                    10 => Uint32,
                    11 => Float32x4,
                    12 => Float32x2,
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
