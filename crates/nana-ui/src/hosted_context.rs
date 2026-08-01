//! Shared WGPU context for a NanaUI hosted application.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::Size;
use iced_wgpu::wgpu;
use iced_winit::winit;

/// Cloneable access to the host's only device and queue pair.
#[derive(Clone)]
pub struct HostedGpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    adapter_info: wgpu::AdapterInfo,
}

impl HostedGpuResources {
    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }
}

/// The single window, surface, adapter, device, and queue used by a hosted app.
pub struct HostedGpuContext {
    window: Arc<winit::window::Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    resources: HostedGpuResources,
    format: wgpu::TextureFormat,
    configuration: wgpu::SurfaceConfiguration,
    device_lost: Arc<AtomicBool>,
}

impl HostedGpuContext {
    /// Creates the only GPU context used by both NanaUI and application renderers.
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Self, HostedGpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| HostedGpuError::SurfaceCreation(error.to_string()))?;
        let adapter = wgpu::util::initialize_adapter_from_env_or_default(&instance, Some(&surface))
            .await
            .map_err(|error| HostedGpuError::Adapter(error.to_string()))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = preferred_surface_format(&capabilities.formats)
            .ok_or(HostedGpuError::SurfaceHasNoFormats)?;
        let alpha_mode = preferred_alpha_mode(&capabilities.alpha_modes);
        let adapter_features = adapter.features();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("NanaUI hosted shared device"),
                required_features: adapter_features & wgpu::Features::default(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .map_err(|error| HostedGpuError::Device(error.to_string()))?;
        let device_lost = Arc::new(AtomicBool::new(false));
        let device_lost_callback = Arc::clone(&device_lost);
        device.set_device_lost_callback(move |_reason, _message| {
            device_lost_callback.store(true, Ordering::Release);
        });

        let size = window.inner_size();
        let configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Srgb,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &configuration);
        let adapter_info = adapter.get_info();

        Ok(Self {
            window,
            instance,
            surface,
            adapter,
            resources: HostedGpuResources {
                device: Arc::new(device),
                queue: Arc::new(queue),
                adapter_info,
            },
            format,
            configuration,
            device_lost,
        })
    }

    pub fn window(&self) -> &Arc<winit::window::Window> {
        &self.window
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub fn resources(&self) -> HostedGpuResources {
        self.resources.clone()
    }

    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn physical_size(&self) -> Size<u32> {
        let size = self.window.inner_size();
        Size::new(size.width, size.height)
    }

    pub fn resize(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.reconfigure();
    }

    pub fn reconfigure(&mut self) {
        self.surface
            .configure(self.resources.device(), &self.configuration);
    }

    pub fn recover_surface(&mut self) -> Result<(), HostedGpuError> {
        let surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(|error| HostedGpuError::SurfaceCreation(error.to_string()))?;
        let capabilities = surface.get_capabilities(&self.adapter);
        if !capabilities.formats.contains(&self.format) {
            return Err(HostedGpuError::SurfaceFormatChanged {
                expected: self.format,
            });
        }
        self.configuration.alpha_mode = preferred_alpha_mode(&capabilities.alpha_modes);
        self.surface = surface;
        self.reconfigure();
        Ok(())
    }

    pub fn is_drawable(&self) -> bool {
        let size = self.window.inner_size();
        size.width > 0 && size.height > 0
    }

    pub fn take_device_lost(&self) -> bool {
        self.device_lost.swap(false, Ordering::AcqRel)
    }

    pub fn acquire_frame(&mut self) -> Result<HostedSurfaceFrame, HostedGpuError> {
        if !self.is_drawable() {
            return Ok(HostedSurfaceFrame::Skipped);
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Ok(HostedSurfaceFrame::Ready(frame)),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.reconfigure();
                Ok(HostedSurfaceFrame::Ready(frame))
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure();
                Ok(HostedSurfaceFrame::Retry)
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recover_surface()?;
                Ok(HostedSurfaceFrame::Retry)
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(HostedSurfaceFrame::Skipped)
            }
            wgpu::CurrentSurfaceTexture::Validation => Err(HostedGpuError::SurfaceValidation),
        }
    }

    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.resources.queue().present(frame);
    }
}

pub enum HostedSurfaceFrame {
    Ready(wgpu::SurfaceTexture),
    Retry,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedGpuError {
    SurfaceCreation(String),
    Adapter(String),
    Device(String),
    SurfaceHasNoFormats,
    SurfaceFormatChanged { expected: wgpu::TextureFormat },
    SurfaceValidation,
}

impl fmt::Display for HostedGpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceCreation(message) => {
                write!(formatter, "failed to create surface: {message}")
            }
            Self::Adapter(message) => write!(formatter, "failed to select GPU adapter: {message}"),
            Self::Device(message) => write!(formatter, "failed to create GPU device: {message}"),
            Self::SurfaceHasNoFormats => formatter.write_str("surface has no texture formats"),
            Self::SurfaceFormatChanged { expected } => write!(
                formatter,
                "recovered surface does not support renderer format {expected:?}"
            ),
            Self::SurfaceValidation => formatter.write_str("surface acquisition failed validation"),
        }
    }
}

impl std::error::Error for HostedGpuError {}

fn preferred_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

fn preferred_alpha_mode(modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Auto,
    ]
    .into_iter()
    .find(|mode| modes.contains(mode))
    .or_else(|| modes.first().copied())
    .unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

#[cfg(test)]
mod tests {
    use super::{preferred_alpha_mode, preferred_surface_format};
    use iced_wgpu::wgpu;

    #[test]
    fn surface_preferences_preserve_transparency_and_srgb() {
        assert_eq!(
            preferred_alpha_mode(&[
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PostMultiplied,
            ]),
            wgpu::CompositeAlphaMode::PostMultiplied
        );
        assert_eq!(
            preferred_surface_format(&[
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ]),
            Some(wgpu::TextureFormat::Bgra8UnormSrgb)
        );
    }
}
