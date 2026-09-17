//! GPU devices for unit tests.
//!
//! Every test gets its own `wgpu::Device`, but the instance and adapter are
//! created once per test process. Creating and dropping whole instances on many
//! test threads at once (the default Vulkan + DX12 backend set on Windows)
//! intermittently deadlocks inside adapter enumeration, device creation or
//! instance teardown, which leaves `cargo test` hanging with every worker
//! stuck in a GPU test.

use std::sync::OnceLock;

fn adapter() -> &'static wgpu::Adapter {
    static ADAPTER: OnceLock<wgpu::Adapter> = OnceLock::new();
    ADAPTER.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("GPU tests require a WGPU adapter")
    })
}

/// A fresh device on the shared test adapter.
pub(crate) fn device(label: &'static str) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter().request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("GPU tests require a WGPU device")
}
