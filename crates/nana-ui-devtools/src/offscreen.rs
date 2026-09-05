//! Snapshot-only offscreen Scene paint + CPU readback.
//!
//! Product windows never use this path. The host owns one Device/Queue.

use std::fs;
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
    image_ready: std::sync::mpsc::Receiver<()>,
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
        let mut painter = SceneWgpuPainter::new(&device, &queue, FORMAT);
        let (wake, image_ready) = std::sync::mpsc::sync_channel(1);
        painter.set_image_waker(std::sync::Arc::new(move || {
            let _ = wake.try_send(());
        }));
        Ok(Self {
            device,
            queue,
            painter,
            image_ready,
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

    /// Paint logical scene coordinates into a physical snapshot at a scale.
    pub fn paint_scaled(
        &mut self,
        scene: &UiScene,
        size: Size<u32>,
        scale_factor: f32,
        clear: [f32; 4],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.paint_layers_scaled(&[(scene, true)], size, scale_factor, clear, None, None)
    }

    pub fn paint_layers(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.paint_layers_scaled(layers, size, 1.0, clear, host_textures, gpu_renderers)
    }

    fn paint_layers_scaled(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        scale_factor: f32,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err("snapshot scale must be finite and positive".into());
        }
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("snapshot timed out waiting for URL images".into());
            }
            // Every attempt starts from the same owned target contents.
            if layers.first().is_none_or(|(_, clears)| !clears) {
                let mut encoder =
                    self.device
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
            let image_revision = self.painter.image_revision();
            for (scene, layer_clear) in layers {
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("nana-ui snapshot paint"),
                        });
                let viewport = ScenePaintViewport {
                    logical_size: [
                        size.width as f32 / scale_factor,
                        size.height as f32 / scale_factor,
                    ],
                    physical_size: [size.width, size.height],
                    scale_factor,
                    scene_origin: [0.0, 0.0],
                    target_origin: [0.0, 0.0],
                    clear_color: if *layer_clear {
                        wgpu_clear(clear)
                    } else {
                        [0.0; 4]
                    },
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
            // Completion during a later layer also requires repainting the earlier
            // layers. Waiting and readback remain confined to this snapshot host.
            if image_revision != self.painter.image_revision() {
                continue;
            }
            if !self.painter.has_pending_images() {
                break;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            self.image_ready
                .recv_timeout(remaining)
                .map_err(|_| "snapshot timed out waiting for URL images")?;
        }
        let copy = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui snapshot copy"),
            });
        readback(&self.device, &self.queue, copy, &texture, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::runtime::{DocumentId, LayoutViewport, RuntimeDocument, Stack};
    use nana_ui_core::{BackgroundImage, BackgroundImageFit, LayoutStyle, LengthSpec};

    #[test]
    fn first_layered_snapshot_waits_for_http_images_and_repaints_overlays() {
        check_layered_snapshot(false);
    }

    #[test]
    fn no_clear_translucent_layers_do_not_accumulate_across_retries() {
        check_layered_snapshot(true);
    }

    fn check_layered_snapshot(no_clear: bool) {
        use std::io::{Read, Write};
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([0, 0, 255, if no_clear { 128 } else { 255 }]),
        )
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!("http://{}/blue.png", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            stream.read(&mut request).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let bytes = png.into_inner();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let mut background = RuntimeDocument::new(DocumentId::new(1).unwrap());
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(64.0)),
            height: Some(LengthSpec::Px(64.0)),
            ..Default::default()
        };
        layout.paint.background_image = Some(BackgroundImage::url_with_fit(
            url,
            BackgroundImageFit::Stretch,
        ));
        background
            .context_mut()
            .create_component(DocumentId::new(1).unwrap(), Stack::from_layout(layout))
            .unwrap();
        let mut overlay = RuntimeDocument::new(DocumentId::new(2).unwrap());
        overlay
            .context_mut()
            .create_component(
                DocumentId::new(2).unwrap(),
                Stack::from_layout(LayoutStyle {
                    width: Some(LengthSpec::Px(16.0)),
                    height: Some(LengthSpec::Px(16.0)),
                    background: Some([1.0, 0.0, 0.0, if no_clear { 0.5 } else { 1.0 }]),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut shaper = nana_ui::NanaTextShaper::default();
        for document in [&mut background, &mut overlay] {
            document
                .flush(LayoutViewport::new(64.0, 64.0), &mut shaper)
                .unwrap();
        }
        let mut gpu = OffscreenSnapshots::new().unwrap();
        let pixels = gpu
            .paint_layers(
                &[(background.scene(), !no_clear), (overlay.scene(), false)],
                Size::new(64, 64),
                [0.0; 4],
                None,
                None,
            )
            .unwrap();
        let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        if no_clear {
            assert!(
                (127..=129).contains(&at(40, 40)[3]),
                "background alpha accumulates: {:?}",
                at(40, 40)
            );
            assert!(
                (190..=193).contains(&at(8, 8)[3]),
                "overlay alpha accumulates: {:?}",
                at(8, 8)
            );
        } else {
            assert!(
                at(40, 40)[2] > 200 && at(40, 40)[0] < 40,
                "first returned snapshot must include the image: {:?}",
                at(40, 40)
            );
            assert!(
                at(8, 8)[0] > 200 && at(8, 8)[2] < 40,
                "overlay must stay above the loaded background"
            );
        }
        server.join().unwrap();
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
    image::save_buffer(
        path,
        pixels,
        size.width,
        size.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(())
}

pub fn read_png(path: &Path) -> Option<(Size<u32>, Vec<u8>)> {
    let image = image::open(path).ok()?.into_rgba8();
    let size = Size::new(image.width(), image.height());
    Some((size, image.into_raw()))
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
