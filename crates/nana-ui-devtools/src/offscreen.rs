//! Snapshot-only offscreen Scene paint + CPU readback.
//!
//! Product windows never use this path. The snapshot owns one `GpuContext`;
//! readback reaches WGPU through `nana_gpu::__framework`, as framework tooling.

use std::fs;
use std::path::{Path, PathBuf};

use nana_gpu::__framework;
use nana_ui::runtime::UiScene;
use nana_ui::{
    GpuContext, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages,
    HostTextureRegistry, SceneGpuRendererRegistry, ScenePaintError, ScenePaintViewport,
    SceneWgpuPainter,
};

/// Physical pixel size for snapshot PNG encode and GPU readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size<T = u32> {
    pub width: T,
    pub height: T,
}

/// Colour primaries carried by a snapshot pixel buffer.
///
/// This is deliberately independent of `wgpu::TextureFormat`: a format says
/// how bytes are stored, while these fields say what those bytes mean.  The
/// offscreen painter currently produces the first variant only, but the
/// additional variants make an attempted HDR export explicit instead of
/// silently treating it as sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPrimaries {
    Srgb,
    DisplayP3,
    Bt2020,
}

/// Transfer function carried by a snapshot pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotTransfer {
    Srgb,
    Linear,
    Bt2100Pq,
    Bt2100Hlg,
}

/// Alpha representation carried by a snapshot pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotAlphaMode {
    Straight,
    /// RGB is premultiplied in linear-light samples before any transfer
    /// encoding. It must be decoded to linear before unpremultiplication.
    Premultiplied,
}

/// Explicit colour contract for CPU pixel buffers.
///
/// This metadata is not inferred from a byte slice. Callers crossing an
/// export boundary must choose the contract explicitly. The typed PNG helper
/// embeds the sRGB contract in the output chunk. The painter's readback is
/// premultiplied sRGB bytes whose RGB decodes to premultiplied linear values,
/// so its metadata is [`Self::PAINTER_READBACK`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotColorMetadata {
    primaries: SnapshotPrimaries,
    transfer: SnapshotTransfer,
    alpha_mode: SnapshotAlphaMode,
}

impl SnapshotColorMetadata {
    pub const fn new(
        primaries: SnapshotPrimaries,
        transfer: SnapshotTransfer,
        alpha_mode: SnapshotAlphaMode,
    ) -> Self {
        Self {
            primaries,
            transfer,
            alpha_mode,
        }
    }

    /// The metadata produced by [`readback_image`] and
    /// [`OffscreenSnapshots::paint_image`].
    pub const PAINTER_READBACK: Self = Self {
        primaries: SnapshotPrimaries::Srgb,
        transfer: SnapshotTransfer::Srgb,
        alpha_mode: SnapshotAlphaMode::Premultiplied,
    };

    /// The only contract accepted by [`write_png_with_metadata`].
    ///
    /// PNG consumers conventionally interpret alpha as straight. Use
    /// [`SnapshotImage::into_png_srgb`] before passing painter output to
    /// this contract; the legacy [`write_png`] helper remains available for
    /// callers that intentionally preserve the historical raw-byte behaviour.
    pub const PNG_SRGB: Self = Self {
        primaries: SnapshotPrimaries::Srgb,
        transfer: SnapshotTransfer::Srgb,
        alpha_mode: SnapshotAlphaMode::Straight,
    };

    pub const fn primaries(self) -> SnapshotPrimaries {
        self.primaries
    }

    pub const fn transfer(self) -> SnapshotTransfer {
        self.transfer
    }

    pub const fn alpha_mode(self) -> SnapshotAlphaMode {
        self.alpha_mode
    }

    /// A BT.2100 PQ buffer.  This is descriptive only; the SDR offscreen
    /// target cannot produce or encode this contract.
    pub const BT2100_PQ: Self = Self {
        primaries: SnapshotPrimaries::Bt2020,
        transfer: SnapshotTransfer::Bt2100Pq,
        alpha_mode: SnapshotAlphaMode::Straight,
    };

    /// A BT.2100 HLG buffer.  This is descriptive only; the SDR offscreen
    /// target cannot produce or encode this contract.
    pub const BT2100_HLG: Self = Self {
        primaries: SnapshotPrimaries::Bt2020,
        transfer: SnapshotTransfer::Bt2100Hlg,
        alpha_mode: SnapshotAlphaMode::Straight,
    };

    pub const fn is_png_srgb(self) -> bool {
        matches!(
            (self.primaries, self.transfer, self.alpha_mode),
            (
                SnapshotPrimaries::Srgb,
                SnapshotTransfer::Srgb,
                SnapshotAlphaMode::Straight
            )
        )
    }
}

/// A pixel buffer together with its size and explicit colour contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotImage {
    size: Size<u32>,
    pixels: Vec<u8>,
    color: SnapshotColorMetadata,
}

impl SnapshotImage {
    pub fn new(
        size: Size<u32>,
        pixels: Vec<u8>,
        color: SnapshotColorMetadata,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let expected = rgba8_len(size)?;
        if pixels.len() != expected {
            return Err(format!(
                "RGBA8 snapshot expects {expected} bytes, got {}",
                pixels.len()
            )
            .into());
        }
        Ok(Self {
            size,
            pixels,
            color,
        })
    }

    pub const fn size(&self) -> Size<u32> {
        self.size
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub const fn color(&self) -> SnapshotColorMetadata {
        self.color
    }

    /// Convert linear-premultiplied SDR sRGB bytes to straight-alpha sRGB PNG
    /// bytes. The conversion decodes RGB, divides by alpha in linear light,
    /// and encodes again; transparent pixels are forced to RGB zero.
    ///
    /// HDR, P3, unknown, straight-alpha, and non-sRGB transfers are rejected.
    /// The byte vector is RGBA8; this API does not claim to convert float HDR
    /// buffers.
    pub fn into_png_srgb(self) -> Result<Self, Box<dyn std::error::Error>> {
        if self.color
            != SnapshotColorMetadata::new(
                SnapshotPrimaries::Srgb,
                SnapshotTransfer::Srgb,
                SnapshotAlphaMode::Premultiplied,
            )
        {
            return Err(format!(
                "PNG sRGB conversion requires premultiplied sRGB bytes, got {:?}",
                self.color
            )
            .into());
        }
        let mut pixels = self.pixels;
        for rgba in pixels.chunks_exact_mut(4) {
            let alpha = f32::from(rgba[3]) / 255.0;
            if alpha <= 0.0 {
                rgba[..3].fill(0);
                continue;
            }
            for channel in &mut rgba[..3] {
                let encoded = f32::from(*channel) / 255.0;
                let linear = srgb_to_linear(encoded) / alpha;
                *channel = linear_to_srgb(linear.clamp(0.0, 1.0));
            }
        }
        SnapshotImage::new(self.size, pixels, SnapshotColorMetadata::PNG_SRGB)
    }
}

impl<T> Size<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

/// The offscreen snapshot target. This API is deliberately SDR-only; HDR
/// presentation formats must use an export path that carries color metadata.
pub const FORMAT: GpuTextureFormat = GpuTextureFormat::BGRA8_UNORM_SRGB;

pub struct OffscreenSnapshots {
    /// The snapshot device. Upload host textures with
    /// [`GpuContext::create_texture`] / [`GpuContext::write_texture`].
    pub gpu: GpuContext,
    painter: SceneWgpuPainter,
    image_ready: std::sync::mpsc::Receiver<()>,
}

impl OffscreenSnapshots {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (_instance, adapter) = request_adapter()?;
        let (raw_device, raw_queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nana-ui snapshot device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))?;
        let gpu = __framework::adopt(adapter, raw_device, raw_queue);
        let mut painter = SceneWgpuPainter::new(&gpu, FORMAT);
        let (wake, image_ready) = std::sync::mpsc::sync_channel(1);
        painter.set_image_waker(std::sync::Arc::new(move || {
            let _ = wake.try_send(());
        }));
        Ok(Self {
            gpu,
            painter,
            image_ready,
        })
    }

    /// Egress for the `http(s)` `url(...)` images of the scenes painted next;
    /// see [`SceneWgpuPainter::set_resource_fetch_host`].
    pub fn set_resource_fetch_host(&mut self, host: Option<nana_ui::SharedFetchHost>) {
        self.painter.set_resource_fetch_host(host);
    }

    /// Product GPU-node renderers. They draw on the device of whichever
    /// painter paints them, here the snapshot's.
    ///
    /// Without them a `GpuView` or `nana.host-texture` node paints nothing in a
    /// screenshot, so an Agent sees a hole where the real application shows
    /// content — and cannot tell that from a genuine layout bug.
    pub fn default_gpu_renderers(&self) -> SceneGpuRendererRegistry {
        nana_ui::default_scene_gpu_renderers()
    }

    pub fn paint(
        &mut self,
        scene: &UiScene,
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(self
            .paint_image(scene, size, clear, host_textures, gpu_renderers)?
            .pixels)
    }

    /// Paint a scene and retain its explicit SDR colour contract.
    pub fn paint_image(
        &mut self,
        scene: &UiScene,
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<SnapshotImage, Box<dyn std::error::Error>> {
        self.paint_layers_scaled_image(
            &[(scene, true)],
            size,
            1.0,
            clear,
            host_textures,
            gpu_renderers,
        )
    }

    /// Paint logical scene coordinates into a physical snapshot at a scale.
    pub fn paint_scaled(
        &mut self,
        scene: &UiScene,
        size: Size<u32>,
        scale_factor: f32,
        clear: [f32; 4],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(self
            .paint_layers_scaled_image(&[(scene, true)], size, scale_factor, clear, None, None)?
            .pixels)
    }

    pub fn paint_layers(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(self
            .paint_layers_scaled_image(layers, size, 1.0, clear, host_textures, gpu_renderers)?
            .pixels)
    }

    /// Physical-scale paint that keeps host textures and GPU renderers.
    /// [`Self::paint_scaled`] drops both, so a product screenshotting real
    /// cover art above 1x needs this entry point rather than its own painter.
    pub fn paint_layers_scaled(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        scale_factor: f32,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(self
            .paint_layers_scaled_image(
                layers,
                size,
                scale_factor,
                clear,
                host_textures,
                gpu_renderers,
            )?
            .pixels)
    }

    /// Physical-scale paint that carries the pixel colour contract alongside
    /// the bytes. Use [`SnapshotImage::into_png_srgb`] before PNG export.
    pub fn paint_layers_scaled_image(
        &mut self,
        layers: &[(&UiScene, bool)],
        size: Size<u32>,
        scale_factor: f32,
        clear: [f32; 4],
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<SnapshotImage, Box<dyn std::error::Error>> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err("snapshot scale must be finite and positive".into());
        }
        if size.width == 0 || size.height == 0 {
            return Err("snapshot size must be non-zero".into());
        }
        let texture = self.gpu.create_texture(&GpuTextureDescriptor {
            label: Some("nana-ui snapshot offscreen"),
            width: size.width,
            height: size.height,
            format: FORMAT,
            usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
        })?;
        let target = texture.render_target()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("snapshot timed out waiting for URL images".into());
            }
            // Every attempt starts from the same owned target contents.
            if layers.first().is_none_or(|(_, clears)| !clears) {
                let mut frame = self.gpu.begin_frame("nana-ui snapshot clear");
                __framework::encoder(&mut frame).begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("nana-ui snapshot clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: __framework::texture_view(&texture),
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
                let clear_submit = frame.submit();
                __framework::device(&self.gpu)
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(
                            __framework::submission_index(&clear_submit).clone(),
                        ),
                        timeout: None,
                    })
                    .map_err(|error| format!("snapshot clear poll failed: {error:?}"))?;
            }
            let image_revision = self.painter.image_revision();
            for (scene, layer_clear) in layers {
                let mut frame = self.gpu.begin_frame("nana-ui snapshot paint");
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
                        &mut frame,
                        &target,
                        viewport,
                        host_textures,
                        gpu_renderers,
                    )
                    .map_err(paint_error)?;
                let paint = frame.submit();
                __framework::device(&self.gpu)
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(__framework::submission_index(&paint).clone()),
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
        readback_image(&self.gpu, &texture, size)
    }
}

/// What a snapshot adapter request found, without building a device.
///
/// Every headless caller asks the same question — "can this machine produce
/// pixel evidence at all?" — so it is answered in one place instead of each
/// test inventing its own skip message.
#[derive(Debug, Clone, Default)]
pub struct GpuProbe {
    pub available: bool,
    pub adapter: Option<String>,
    pub backend: Option<String>,
    /// Why no adapter was found. `None` when one was.
    pub reason: Option<String>,
}

fn request_adapter() -> Result<(wgpu::Instance, wgpu::Adapter), Box<dyn std::error::Error>> {
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
    Ok((instance, adapter))
}

/// Report the snapshot adapter without creating a device or a painter.
pub fn gpu_probe() -> GpuProbe {
    match request_adapter() {
        Ok((_instance, adapter)) => {
            let info = adapter.get_info();
            GpuProbe {
                available: true,
                adapter: Some(info.name),
                backend: Some(format!("{:?}", info.backend)),
                reason: None,
            }
        }
        Err(error) => GpuProbe {
            available: false,
            adapter: None,
            backend: None,
            reason: Some(error.to_string()),
        },
    }
}

/// `true` when a snapshot adapter exists, after one canonical skip line otherwise.
///
/// Use this when the pixels come from a session that builds its own GPU; use
/// [`optional`] when the caller needs the [`OffscreenSnapshots`] itself.
pub fn pixels_available() -> bool {
    let probe = gpu_probe();
    if !probe.available {
        eprintln!(
            "skipping offscreen GPU evidence: {}",
            probe.reason.as_deref().unwrap_or("no snapshot adapter")
        );
    }
    probe.available
}

/// Snapshot GPU, or `None` after one canonical skip line on stderr.
///
/// A GPU-less environment must skip *visibly*: a silent skip reads exactly like
/// a pass, and a report claiming pixel evidence would then be unfalsifiable.
pub fn optional() -> Option<OffscreenSnapshots> {
    match OffscreenSnapshots::new() {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping offscreen GPU evidence: {error}");
            None
        }
    }
}

/// Copy the SDR `FORMAT` texture back to the CPU as RGBA8 sRGB rows.
/// Snapshot tooling only: the product path never reads pixels back. This
/// compatibility helper returns the bytes only; use [`readback_image`] when
/// crossing an export boundary so the colour contract cannot be dropped.
pub fn readback(
    gpu: &GpuContext,
    texture: &GpuTexture,
    size: Size<u32>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(readback_image(gpu, texture, size)?.pixels)
}

/// Copy the SDR `FORMAT` texture back to the CPU with an explicit colour
/// contract. The returned bytes are sRGB-transfer RGBA8 with premultiplied
/// alpha, matching the painter's presentation target.
pub fn readback_image(
    gpu: &GpuContext,
    texture: &GpuTexture,
    size: Size<u32>,
) -> Result<SnapshotImage, Box<dyn std::error::Error>> {
    if size.width == 0 || size.height == 0 {
        return Err("snapshot readback size must be non-zero".into());
    }
    if texture.generation() != gpu.generation() {
        return Err("snapshot readback texture belongs to a different GPU device".into());
    }
    if texture.format() != FORMAT {
        return Err(format!(
            "snapshot readback requires {:?}, got {:?}",
            FORMAT,
            texture.format()
        )
        .into());
    }
    if !texture.usage().contains(GpuTextureUsages::COPY_SRC) {
        return Err("snapshot readback texture must include COPY_SRC usage".into());
    }
    if texture.size() != (size.width, size.height) {
        return Err(format!(
            "snapshot readback extent mismatch: texture {:?}, requested {:?}",
            texture.size(),
            (size.width, size.height)
        )
        .into());
    }
    let device = __framework::device(gpu);
    let mut frame = gpu.begin_frame("nana-ui snapshot copy");
    let unpadded = rgba8_len(Size::new(size.width, 1))?;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded
        .checked_add(alignment - 1)
        .ok_or("snapshot readback row layout overflow")?
        / alignment
        * alignment;
    if padded > u32::MAX as usize {
        return Err("snapshot readback row pitch exceeds WGPU's u32 limit".into());
    }
    let buffer_size = padded
        .checked_mul(size.height as usize)
        .ok_or("snapshot readback buffer size overflow")?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nana-ui snapshot readback"),
        size: buffer_size as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    __framework::encoder(&mut frame).copy_texture_to_buffer(
        __framework::texture(texture).as_image_copy(),
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
    let submission = frame.submit();
    let slice = buffer.slice(..);
    let (map_done, map_result) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = map_done.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(__framework::submission_index(&submission).clone()),
            timeout: None,
        })
        .map_err(|error| format!("snapshot readback poll failed: {error:?}"))?;
    map_result
        .recv()
        .map_err(|error| format!("snapshot readback map callback failed: {error}"))?
        .map_err(|error| format!("snapshot readback map failed: {error:?}"))?;
    let mapped = slice
        .get_mapped_range()
        .expect("snapshot readback buffer must be mapped");
    let mut pixels = Vec::with_capacity(unpadded * size.height as usize);
    for row in mapped.chunks_exact(padded) {
        for pixel in row[..unpadded].as_chunks::<4>().0 {
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    drop(mapped);
    buffer.unmap();
    SnapshotImage::new(size, pixels, SnapshotColorMetadata::PAINTER_READBACK)
}

/// Write already-encoded SDR RGBA8 sRGB pixels as a PNG.
///
/// The byte slice must contain exactly `width * height * 4` bytes. HDR or
/// linear-float bytes must be converted through an explicit export path before
/// calling this function.
pub fn write_png(
    path: &Path,
    size: Size<u32>,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let expected = rgba8_len(size)?;
    if pixels.len() != expected {
        return Err(format!(
            "SDR RGBA8 PNG expects {expected} bytes, got {}",
            pixels.len()
        )
        .into());
    }
    write_png_bytes(path, size, pixels)
}

/// Write an SDR sRGB PNG from a buffer with an explicit colour contract.
///
/// PNG output is accepted only for straight-alpha sRGB bytes. In particular,
/// passing [`SnapshotColorMetadata::BT2100_PQ`], HLG, linear, or premultiplied
/// pixels fails instead of silently producing a misleading SDR file. The
/// metadata is validated here because PNG does not carry this contract in a
/// portable way. Use [`write_png`] only for the legacy raw-byte path.
pub fn write_png_with_metadata(
    path: &Path,
    image: &SnapshotImage,
) -> Result<(), Box<dyn std::error::Error>> {
    if !image.color.is_png_srgb() {
        return Err(format!(
            "PNG export requires straight-alpha sRGB metadata, got {:?}",
            image.color
        )
        .into());
    }
    let expected = rgba8_len(image.size)?;
    if image.pixels.len() != expected {
        return Err(format!(
            "SDR RGBA8 PNG expects {expected} bytes, got {}",
            image.pixels.len()
        )
        .into());
    }
    write_png_srgb_bytes(path, image.size, &image.pixels)
}

/// Convert the offscreen painter's premultiplied readback into straight-alpha
/// sRGB and write a metadata-tagged PNG. This is the safe export helper for
/// callers that still expose the historical `(Size, Vec<u8>)` screenshot API.
pub fn write_painter_png(
    path: &Path,
    size: Size<u32>,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let image = SnapshotImage::new(
        size,
        pixels.to_vec(),
        SnapshotColorMetadata::PAINTER_READBACK,
    )?
    .into_png_srgb()?;
    write_png_with_metadata(path, &image)
}

/// Read an SDR PNG as RGBA8 sRGB bytes with an explicit metadata contract.
/// Unsupported ICC, cICP/HDR, gamma, and chromaticity chunks are rejected;
/// untagged PNGs are rejected by this typed API because their colour meaning
/// cannot be proven. Use [`read_png`] for the legacy untagged compatibility
/// path.
pub fn read_png_with_metadata(path: &Path) -> Option<SnapshotImage> {
    read_png_with_metadata_result(path).ok()
}

/// Fallible form of [`read_png_with_metadata`] with a reason for rejection.
pub fn read_png_with_metadata_result(
    path: &Path,
) -> Result<SnapshotImage, Box<dyn std::error::Error>> {
    use std::io::BufReader;
    let file = fs::File::open(path)?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let reader = decoder.read_info()?;
    let info = reader.info();
    let cicp_is_srgb = info.coding_independent_code_points.is_some_and(|cicp| {
        cicp.color_primaries == 1
            && cicp.transfer_function == 13
            && cicp.matrix_coefficients == 0
            && cicp.is_video_full_range_image
    });
    if info.srgb.is_none() && !cicp_is_srgb {
        return Err("typed PNG reader requires explicit sRGB metadata".into());
    }
    if info.icc_profile.is_some()
        || info.gama_chunk.is_some()
        || info.chrm_chunk.is_some()
        || info.mastering_display_color_volume.is_some()
        || info.content_light_level.is_some()
    {
        return Err("typed PNG reader rejects non-sRGB colour metadata".into());
    }
    if info.coding_independent_code_points.is_some() && !cicp_is_srgb {
        return Err("typed PNG reader rejects non-sRGB cICP metadata".into());
    }
    drop(reader);
    let decoded = image::open(path)?.into_rgba8();
    let size = Size::new(decoded.width(), decoded.height());
    SnapshotImage::new(size, decoded.into_raw(), SnapshotColorMetadata::PNG_SRGB)
}

/// Read an SDR PNG as RGBA8 sRGB bytes. No HDR metadata is inferred.
pub fn read_png(path: &Path) -> Option<(Size<u32>, Vec<u8>)> {
    let image = image::open(path).ok()?.into_rgba8();
    let size = Size::new(image.width(), image.height());
    Some((size, image.into_raw()))
}

fn write_png_bytes(
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

fn write_png_srgb_bytes(
    path: &Path,
    size: Size<u32>,
    pixels: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufWriter;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    Ok(())
}

fn rgba8_len(size: Size<u32>) -> Result<usize, Box<dyn std::error::Error>> {
    (size.width as usize)
        .checked_mul(size.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("snapshot dimensions overflow RGBA8 byte count: {size:?}").into())
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
    let pixels = snapshots
        .paint_image(scene, size, clear, host_textures, gpu_renderers)?
        .into_png_srgb()?;
    let path = output.join(name);
    write_png_with_metadata(&path, &pixels)?;
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

fn linear_to_srgb(u: f32) -> u8 {
    let encoded = if u <= 0.0031308 {
        u * 12.92
    } else {
        1.055 * u.powf(1.0 / 2.4) - 0.055
    };
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::runtime::{DocumentId, LayoutViewport, RuntimeDocument, Stack};
    use nana_ui_core::{BackgroundImage, BackgroundImageFit, LayoutStyle, LengthSpec};

    #[test]
    fn snapshot_metadata_rejects_non_sdr_png_exports() {
        let image = SnapshotImage::new(
            Size::new(1, 1),
            vec![255, 0, 0, 255],
            SnapshotColorMetadata::BT2100_PQ,
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "nana-ui-devtools-hdr-export-{}-{}.png",
            std::process::id(),
            unique_test_suffix()
        ));
        let error = write_png_with_metadata(&path, &image).unwrap_err();
        assert!(error.to_string().contains("straight-alpha sRGB"));
        assert!(!path.exists());
    }

    #[test]
    fn snapshot_metadata_round_trips_sdr_png_contract() {
        let path = std::env::temp_dir().join(format!(
            "nana-ui-devtools-sdr-export-{}-{}.png",
            std::process::id(),
            unique_test_suffix()
        ));
        let image = SnapshotImage::new(
            Size::new(2, 1),
            vec![255, 0, 0, 255, 0, 128, 255, 200],
            SnapshotColorMetadata::PNG_SRGB,
        )
        .unwrap();
        write_png_with_metadata(&path, &image).unwrap();
        let decoded = read_png_with_metadata(&path).expect("PNG must decode");
        assert_eq!(decoded.size, image.size);
        assert_eq!(decoded.pixels, image.pixels);
        assert_eq!(decoded.color, SnapshotColorMetadata::PNG_SRGB);
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let reader = decoder.read_info().unwrap();
        assert!(reader.info().srgb.is_some(), "typed PNG must carry sRGB");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_untagged_png_stays_compatible_but_typed_reader_rejects_it() {
        let path = std::env::temp_dir().join(format!(
            "nana-ui-devtools-legacy-png-{}-{}.png",
            std::process::id(),
            unique_test_suffix()
        ));
        write_png(&path, Size::new(1, 1), &[1, 2, 3, 255]).unwrap();
        assert!(read_png(&path).is_some());
        assert!(read_png_with_metadata(&path).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn into_png_srgb_unpremultiplies_in_linear_light_and_clears_transparent_rgb() {
        // sRGB(0.5) is about 188; after linear decode and division by 0.5 it
        // represents opaque red again and should encode back to 255.
        let image = SnapshotImage::new(
            Size::new(2, 1),
            vec![188, 0, 0, 128, 55, 66, 77, 0],
            SnapshotColorMetadata::PAINTER_READBACK,
        )
        .unwrap()
        .into_png_srgb()
        .unwrap();
        assert_eq!(image.color(), SnapshotColorMetadata::PNG_SRGB);
        assert!(image.pixels()[0] >= 254);
        assert_eq!(&image.pixels()[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn into_png_srgb_rejects_hdr_and_wide_gamut_contracts() {
        for color in [
            SnapshotColorMetadata::BT2100_PQ,
            SnapshotColorMetadata::BT2100_HLG,
            SnapshotColorMetadata::new(
                SnapshotPrimaries::DisplayP3,
                SnapshotTransfer::Srgb,
                SnapshotAlphaMode::Premultiplied,
            ),
            SnapshotColorMetadata::new(
                SnapshotPrimaries::Srgb,
                SnapshotTransfer::Linear,
                SnapshotAlphaMode::Premultiplied,
            ),
        ] {
            let image = SnapshotImage::new(Size::new(1, 1), vec![0; 4], color).unwrap();
            assert!(image.into_png_srgb().is_err());
        }
    }

    #[test]
    fn snapshot_image_rejects_short_rgba8_buffers() {
        let error = SnapshotImage::new(
            Size::new(2, 2),
            vec![0; 15],
            SnapshotColorMetadata::PNG_SRGB,
        )
        .unwrap_err();
        assert!(error.to_string().contains("expects 16 bytes"));
    }

    #[test]
    fn readback_rejects_invalid_format_usage_and_extent_before_gpu_copy() {
        let gpu = OffscreenSnapshots::new().unwrap();
        let wrong_format = gpu
            .gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("invalid readback format"),
                width: 1,
                height: 1,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::COPY_SRC,
            })
            .unwrap();
        assert!(
            readback_image(&gpu.gpu, &wrong_format, Size::new(1, 1))
                .unwrap_err()
                .to_string()
                .contains("requires")
        );

        let wrong_usage = gpu
            .gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("invalid readback usage"),
                width: 1,
                height: 1,
                format: FORMAT,
                usage: GpuTextureUsages::RENDER_TARGET,
            })
            .unwrap();
        assert!(
            readback_image(&gpu.gpu, &wrong_usage, Size::new(1, 1))
                .unwrap_err()
                .to_string()
                .contains("COPY_SRC")
        );

        let wrong_extent = gpu
            .gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("invalid readback extent"),
                width: 2,
                height: 2,
                format: FORMAT,
                usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
            })
            .unwrap();
        assert!(
            readback_image(&gpu.gpu, &wrong_extent, Size::new(1, 1))
                .unwrap_err()
                .to_string()
                .contains("extent mismatch")
        );
    }

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos()
    }

    #[test]
    fn first_layered_snapshot_waits_for_http_images_and_repaints_overlays() {
        check_layered_snapshot(false);
    }

    #[test]
    fn no_clear_translucent_layers_do_not_accumulate_across_retries() {
        check_layered_snapshot(true);
    }

    /// Test double authorizing any `127.0.0.1` origin: each test binds port 0,
    /// so no exact-match policy can be written before the port is known.
    /// Mirrors the one in `nana-ui`, which is `cfg(test)`-only and therefore
    /// not reachable from here.
    #[derive(Debug)]
    struct LoopbackFetchHost {
        policy: nana_ui::FetchPolicy,
    }

    impl nana_ui::FetchHost for LoopbackFetchHost {
        fn fetch(
            &self,
            request: nana_ui::FetchRequest,
        ) -> Result<nana_ui::FetchResponse, nana_ui::FetchError> {
            let authority = request
                .url
                .strip_prefix("http://")
                .and_then(|rest| rest.split('/').next())
                .filter(|authority| authority.split(':').next() == Some("127.0.0.1"))
                .ok_or_else(|| {
                    nana_ui::FetchError::new(
                        nana_ui::FetchErrorKind::Policy,
                        format!("test host serves loopback only: `{}`", request.url),
                    )
                })?;
            let policy = nana_ui::FetchPolicy::default()
                .with_allowed_origin(&format!("http://{authority}"))?;
            nana_ui::NativeFetchHost::new(policy).fetch(request)
        }

        fn policy(&self) -> &nana_ui::FetchPolicy {
            &self.policy
        }
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
            // The stub answers every request identically; the byte count and a
            // client that closes early are both uninteresting here.
            let _ = stream.read(&mut request);
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
        gpu.set_resource_fetch_host(Some(nana_ui::shared_fetch_host(LoopbackFetchHost {
            policy: nana_ui::FetchPolicy::default(),
        })));
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
