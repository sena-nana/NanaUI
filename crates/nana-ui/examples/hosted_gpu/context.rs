use iced::Size;
use iced_wgpu::wgpu;
use iced_winit::futures::futures::executor;
use iced_winit::winit;

use std::sync::Arc;

pub struct HostGraphics {
    instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub format: wgpu::TextureFormat,
    configuration: wgpu::SurfaceConfiguration,
}

impl HostGraphics {
    pub fn new(window: Arc<winit::window::Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(window.clone())
            .expect("host window must support a WGPU surface");
        let adapter = executor::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance,
            Some(&surface),
        ))
        .expect("host must find a surface-compatible WGPU adapter");
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .expect("surface must expose at least one texture format");
        let alpha_mode = preferred_alpha_mode(&capabilities.alpha_modes);
        let adapter_features = adapter.features();
        let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-ui host device"),
            required_features: adapter_features & wgpu::Features::default(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("host must create the shared WGPU device");

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
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &configuration);

        Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            format,
            configuration,
        }
    }

    pub fn resize(&mut self, window: &winit::window::Window) {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.surface.configure(&self.device, &self.configuration);
    }

    pub fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.configuration);
    }

    pub fn recover_surface(&mut self, window: Arc<winit::window::Window>) {
        self.surface = self
            .instance
            .create_surface(window)
            .expect("host window must remain compatible with the WGPU instance");
        self.reconfigure();
    }

    pub fn is_drawable(&self) -> bool {
        self.configuration.width > 0 && self.configuration.height > 0
    }

    pub fn physical_size(&self) -> Size<u32> {
        Size::new(self.configuration.width, self.configuration.height)
    }
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
    use super::preferred_alpha_mode;
    use iced_wgpu::wgpu;

    #[test]
    fn alpha_mode_prefers_compositor_friendly_modes_with_a_safe_fallback() {
        assert_eq!(
            preferred_alpha_mode(&[
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PostMultiplied,
            ]),
            wgpu::CompositeAlphaMode::PostMultiplied
        );
        assert_eq!(
            preferred_alpha_mode(&[wgpu::CompositeAlphaMode::Opaque]),
            wgpu::CompositeAlphaMode::Opaque
        );
        assert_eq!(preferred_alpha_mode(&[]), wgpu::CompositeAlphaMode::Auto);
    }
}
