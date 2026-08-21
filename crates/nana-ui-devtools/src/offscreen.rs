//! Snapshot-only offscreen Scene paint + CPU readback.
//!
//! Product windows never use this path. The host owns one Device/Queue.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use nana_ui::runtime::UiScene;
use nana_ui::{
    HostTextureRegistry, SceneGpuRendererRegistry, ScenePaintError, ScenePaintViewport,
    SceneWgpuPainter,
};

/// Physical pixel size for snapshot PNG encode and GPU readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size<T = u32> {
    pub width: T,
    pub height: T,
}

impl<T> Size<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

pub struct OffscreenSnapshots {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    painter: SceneWgpuPainter,
}

impl OffscreenSnapshots {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nana-ui snapshot device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))?;
        let painter = SceneWgpuPainter::new(&device, &queue, FORMAT);
        Ok(Self {
            device,
            queue,
            painter,
        })
    }

    pub fn paint(
        &mut self,
        scene: &UiScene,
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.paint_layers(&[(scene, true)], size, clear, host_textures, gpu_renderers)
    }

    pub fn paint_layers(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if size.width == 0 || size.height == 0 {
            return Err("snapshot size must be non-zero".into());
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui snapshot offscreen"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        if layers.is_empty() {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui snapshot clear"),
                });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui snapshot clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu_clear_color(clear)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let clear_submit = self.queue.submit([encoder.finish()]);
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(clear_submit),
                    timeout: None,
                })
                .map_err(|error| format!("snapshot clear poll failed: {error:?}"))?;
        }
        for (scene, layer_clear) in layers {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui snapshot paint"),
                });
            let viewport = ScenePaintViewport {
                logical_size: [size.width as f32, size.height as f32],
                physical_size: [size.width, size.height],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color: wgpu_clear(clear),
                clear: *layer_clear,
            };
            self.painter
                .paint(
                    scene,
                    &mut encoder,
                    &view,
                    viewport,
                    host_textures,
                    gpu_renderers,
                )
                .map_err(paint_error)?;
            let paint = self.queue.submit([encoder.finish()]);
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(paint),
                    timeout: None,
                })
                .map_err(|error| format!("snapshot paint poll failed: {error:?}"))?;
        }
        let copy = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui snapshot copy"),
            });
        readback(&self.device, &self.queue, copy, &texture, size)
    }
}

pub fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    size: Size<u32>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let unpadded = size.width as usize * 4;
    let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nana-ui snapshot readback"),
        size: (padded * size.height as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(size.height),
            },
        },
        wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|error| format!("snapshot readback poll failed: {error:?}"))?;
    let mapped = slice
        .get_mapped_range()
        .expect("snapshot readback buffer must be mapped");
    let mut pixels = Vec::with_capacity(unpadded * size.height as usize);
    for row in mapped.chunks(padded) {
        for pixel in row[..unpadded].chunks_exact(4) {
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}

pub fn write_png(
    path: &Path,
    size: Size<u32>,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    Ok(())
}

pub fn read_png(path: &Path) -> Option<(Size<u32>, Vec<u8>)> {
    let file = File::open(path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let raw = &buf[..info.buffer_size()];
    let pixels = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => raw
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        _ => return None,
    };
    Some((Size::new(info.width, info.height), pixels))
}

#[allow(clippy::too_many_arguments)]
pub fn write_scene(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    scene: &UiScene,
    size: Size<u32>,
    clear: [f32; 4],
    host_textures: Option<&HostTextureRegistry>,
    gpu_renderers: Option<&SceneGpuRendererRegistry>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let pixels = snapshots.paint(scene, size, clear, host_textures, gpu_renderers)?;
    let path = output.join(name);
    write_png(&path, size, &pixels)?;
    Ok(path)
}

fn paint_error(error: ScenePaintError) -> Box<dyn std::error::Error> {
    Box::new(error)
}

/// `ScenePaintViewport.clear_color` is consumed as `wgpu::Color` (linear).
fn wgpu_clear([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a]
}

fn wgpu_clear_color(clear: [f32; 4]) -> wgpu::Color {
    let [r, g, b, a] = wgpu_clear(clear);
    wgpu::Color {
        r: r as f64,
        g: g as f64,
        b: b as f64,
        a: a as f64,
    }
}

fn srgb_to_linear(u: f32) -> f32 {
    if u < 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}
