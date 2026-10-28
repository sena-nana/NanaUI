//! The one Device/Queue a mobile host keeps, and the surface it presents to.

use wgpu::{
    Adapter, CompositeAlphaMode, CurrentSurfaceTexture, Instance, PresentMode, Surface,
    SurfaceConfiguration, SurfaceTargetUnsafe, TextureFormat, TextureUsages,
};

use nana_ui::{
    GpuContext, GpuRenderTarget, GpuTextureFormat, HostTextureRegistry, ScenePaintViewport,
    SceneWgpuPainter,
};

use crate::runtime::MobileRuntime;

/// How the surface composites with the native layers around it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SurfaceOptions {
    /// Premultiplied alpha instead of opaque, for a native layer under the
    /// NanaUI surface (a camera preview): what the document leaves
    /// transparent shows the layer below. Costs the system compositor a
    /// blend, so opaque stays the default.
    pub transparent: bool,
}

struct AttachedSurface {
    surface: Surface<'static>,
    config: SurfaceConfiguration,
}

/// Device, painter and host textures, kept for the life of the app; the
/// surface comes and goes with the native window.
pub struct MobileRenderer {
    instance: Instance,
    adapter: Adapter,
    gpu: GpuContext,
    painter: SceneWgpuPainter,
    format: TextureFormat,
    options: SurfaceOptions,
    surface: Option<AttachedSurface>,
    host_textures: HostTextureRegistry,
    painted_host_revision: u64,
}

impl MobileRenderer {
    /// Create the device on the first surface.
    ///
    /// # Safety
    ///
    /// `target` must stay valid until [`Self::detach`] or drop: the
    /// `CAMetalLayer` or `ANativeWindow` it names is retained by the shell
    /// while the surface exists.
    pub unsafe fn new(
        backends: wgpu::Backends,
        target: SurfaceTargetUnsafe,
        physical_size: (u32, u32),
        options: SurfaceOptions,
    ) -> Result<Self, String> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // SAFETY: forwarded from the caller.
        let surface = unsafe { instance.create_surface_unsafe(target) }
            .map_err(|error| format!("create_surface: {error}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|error| format!("request_adapter: {error}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-mobile-host"),
            required_features: wgpu::Features::empty(),
            // SceneWgpuPainter exceeds downlevel limits (fragment storage
            // buffers, 16 inter-stage variables); ask for what the adapter
            // has, never more.
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .map_err(|error| format!("request_device: {error}"))?;
        let gpu = GpuContext::from_wgpu(adapter.clone(), device, queue);
        let config = surface_config(&surface, &adapter, physical_size, options)?;
        surface.configure(gpu.wgpu().device(), &config);
        let format = config.format;
        Ok(Self {
            painter: SceneWgpuPainter::new(&gpu, GpuTextureFormat::from_wgpu(format)),
            instance,
            adapter,
            gpu,
            format,
            options,
            surface: Some(AttachedSurface { surface, config }),
            host_textures: HostTextureRegistry::new(),
            painted_host_revision: 0,
        })
    }

    /// A new native window after [`Self::detach`]: same device, same
    /// painter and host textures.
    ///
    /// # Safety
    ///
    /// As [`Self::new`].
    pub unsafe fn attach(
        &mut self,
        target: SurfaceTargetUnsafe,
        physical_size: (u32, u32),
    ) -> Result<(), String> {
        // SAFETY: forwarded from the caller.
        let surface = unsafe { self.instance.create_surface_unsafe(target) }
            .map_err(|error| format!("create_surface: {error}"))?;
        let config = surface_config(&surface, &self.adapter, physical_size, self.options)?;
        surface.configure(self.gpu.wgpu().device(), &config);
        if config.format != self.format {
            self.format = config.format;
            self.painter =
                SceneWgpuPainter::new(&self.gpu, GpuTextureFormat::from_wgpu(self.format));
        }
        self.surface = Some(AttachedSurface { surface, config });
        Ok(())
    }

    /// The native window is going away (background, `TerminateWindow`).
    pub fn detach(&mut self) {
        self.surface = None;
    }

    pub fn is_attached(&self) -> bool {
        self.surface.is_some()
    }

    pub fn resize(&mut self, physical_size: (u32, u32)) {
        let Some(attached) = &mut self.surface else {
            return;
        };
        let (width, height) = (physical_size.0.max(1), physical_size.1.max(1));
        if (attached.config.width, attached.config.height) == (width, height) {
            return;
        }
        attached.config.width = width;
        attached.config.height = height;
        attached
            .surface
            .configure(self.gpu.wgpu().device(), &attached.config);
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// Textures the application produces (camera preview) and the document
    /// shows through `HostTexture` nodes.
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }

    /// Whether a host texture changed since the last present.
    pub fn host_textures_changed(&self) -> bool {
        self.host_textures.revision() != self.painted_host_revision
    }

    /// Paint the document's scene into the next surface texture and present.
    pub fn render(&mut self, runtime: &MobileRuntime) -> Result<(), String> {
        let Some(attached) = &mut self.surface else {
            return Ok(());
        };
        let frame = match attached.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) | CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return Ok(()),
            CurrentSurfaceTexture::Outdated => {
                attached
                    .surface
                    .configure(self.gpu.wgpu().device(), &attached.config);
                return Err("surface outdated (reconfigured)".into());
            }
            CurrentSurfaceTexture::Lost => return Err("surface lost".into()),
            CurrentSurfaceTexture::Validation => return Err("surface validation".into()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let physical = [frame.texture.width(), frame.texture.height()];
        let target = GpuRenderTarget::from_wgpu(&self.gpu, view, self.format, physical);
        let viewport = runtime.viewport();
        let (logical_w, logical_h) = viewport.logical_size();
        let paint_viewport = ScenePaintViewport {
            logical_size: [logical_w, logical_h],
            physical_size: physical,
            scale_factor: viewport.scale,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 0.0],
            clear: true,
        };
        let revision = self.host_textures.revision();
        let mut recording = self.gpu.begin_frame("nana-mobile frame");
        self.painter
            .paint(
                runtime.document().scene(),
                &mut recording,
                &target,
                paint_viewport,
                Some(&self.host_textures),
                None,
            )
            .map_err(|error| format!("paint: {error}"))?;
        recording.submit();
        self.gpu.wgpu().queue().present(frame);
        self.painted_host_revision = revision;
        Ok(())
    }
}

fn surface_config(
    surface: &Surface<'_>,
    adapter: &Adapter,
    physical_size: (u32, u32),
    options: SurfaceOptions,
) -> Result<SurfaceConfiguration, String> {
    let caps = surface.get_capabilities(adapter);
    // Take the first format the table lists in sRGB rather than the first
    // format: wgpu 30 can advertise formats only valid in another colour
    // space.
    let (format, color_space) = caps
        .format_capabilities
        .iter()
        .find_map(|row| {
            row.color_spaces
                .contains(wgpu::SurfaceColorSpaces::SRGB)
                .then_some((row.format, wgpu::SurfaceColorSpace::Srgb))
        })
        .or_else(|| {
            caps.formats
                .first()
                .map(|format| (*format, wgpu::SurfaceColorSpace::Srgb))
        })
        .ok_or_else(|| "no surface formats".to_string())?;
    // Metal offers only Opaque and PostMultiplied, and the latter is what
    // clears `CAMetalLayer.opaque`: Core Animation composites the layer
    // premultiplied either way. Falling back to Opaque would paint every
    // transparent pixel black.
    let wanted: &[CompositeAlphaMode] = if options.transparent {
        &[
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::Inherit,
        ]
    } else {
        &[CompositeAlphaMode::Opaque]
    };
    let alpha_mode = wanted
        .iter()
        .copied()
        .find(|mode| caps.alpha_modes.contains(mode))
        .or_else(|| caps.alpha_modes.first().copied())
        .unwrap_or(CompositeAlphaMode::Auto);
    // Fifo is vsync-paced and always available; the display link decides
    // which vsyncs present at all.
    let present_mode = if caps.present_modes.contains(&PresentMode::Fifo) {
        PresentMode::Fifo
    } else {
        caps.present_modes
            .first()
            .copied()
            .unwrap_or(PresentMode::Fifo)
    };
    Ok(SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space,
        width: physical_size.0.max(1),
        height: physical_size.1.max(1),
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    })
}
