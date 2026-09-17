//! GPU device for unit tests.
//!
//! The whole test process shares one `wgpu::Device`, created on first use.
//! Creating devices while tests run deadlocks on Windows with the NVIDIA Vulkan
//! ICD: device creation loads libraries under the driver's lock while
//! libtest's per-test threads exit under the loader lock, and `cargo test`
//! hangs with every worker stuck in a GPU test. Tests only rely on their own
//! painters, targets and resources, never on owning the device.

use std::sync::OnceLock;

/// The process-wide test device and queue.
pub(crate) fn device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().unwrap_or_default(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
                &instance, None,
            ))
            .expect("GPU tests require a WGPU adapter");
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nana-ui test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))
            .expect("GPU tests require a WGPU device")
        })
        .clone()
}
